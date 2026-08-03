use anyhow::{Context, Result};

use super::menu::{popup_viewport, strip_pad, GumMenu, Menu};
use super::{agent, FlowResult, Outcome};
use crate::config::{blank_tab_layout, Config, TabLayout, BLANK_LAYOUT_NAME};
use crate::herdr::HerdrClient;

const TITLE: &str = "\u{ebeb}  Tab Layout"; // nf-cod-window
const SUBTITLE: &str = "Choose a layout.";
const HEIGHT: u8 = 8;

/// The menu rows, in order: `default_tab_layout` first, the rest in config order, and the
/// blank layout last.
///
/// The blank entry is appended whatever the config says, so a config with no `tab_layouts`
/// — or none the user wants right now — can still open a plain tab. A config layout named
/// [`BLANK_LAYOUT_NAME`] replaces the built-in body but keeps the last slot.
fn ordered_layouts(config: &Config) -> Result<Vec<TabLayout>> {
    let default = agent::resolve_layout(config, None)?;
    let mut ordered = std::iter::once(default)
        .chain(
            config
                .tab_layouts
                .iter()
                .filter(|layout| layout.name != config.default_tab_layout),
        )
        .cloned()
        .collect::<Vec<_>>();

    let blank = ordered
        .iter()
        .position(|layout| layout.name == BLANK_LAYOUT_NAME)
        .map(|index| ordered.remove(index))
        .unwrap_or_else(blank_tab_layout);
    ordered.push(blank);
    Ok(ordered)
}

/// Layouts are returned owned because the blank entry may not exist in the config at all.
fn choose_layout(config: &Config, menu: &mut impl Menu) -> Result<Option<TabLayout>> {
    let mut ordered = ordered_layouts(config)?;
    if ordered.len() == 1 {
        return Ok(ordered.pop());
    }

    // The rows and the layouts they were rendered from stay side by side, so the selection
    // resolves by position instead of re-rendering every label a second time. An answer
    // that matches no row — empty, or anything gum returned that is not a choice — falls
    // out as a cancel rather than a panic.
    let options = ordered
        .iter()
        .map(|layout| layout.menu_label())
        .collect::<Vec<_>>();
    let Some(selection) = menu.choose(TITLE, SUBTITLE, &options, HEIGHT)? else {
        return Ok(None);
    };
    let selection = strip_pad(&selection);

    Ok(options
        .iter()
        .position(|option| *option == selection)
        .map(|index| ordered.swap_remove(index)))
}

pub fn new(client: &dyn HerdrClient) -> FlowResult {
    let config = Config::load().context("failed to load config")?;
    let (cols, lines) = popup_viewport();
    let mut menu = GumMenu::new(cols, lines);
    new_with(client, &config, &mut menu)
}

// The layout menu draws before anything touches the socket: it does not depend on the
// project directory, and popup_with_layout adopts the invoking pane's cwd with a pane.get.
// Cancelling here therefore issues no request at all.
fn new_with(client: &dyn HerdrClient, config: &Config, menu: &mut impl Menu) -> FlowResult {
    let Some(layout) = choose_layout(config, menu)? else {
        return Ok(Outcome::Cancelled);
    };
    agent::popup_with_layout(client, config, &layout, false)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};

    use anyhow::Result;
    use serde_json::json;

    use super::*;
    use crate::config::{LayoutPane, PaneType};
    use crate::flows::menu::InputIndent;
    use crate::herdr::FakeClient;

    struct FakeMenu {
        answers: VecDeque<Option<String>>,
        options: Vec<Vec<String>>,
    }

    impl FakeMenu {
        fn new<'a>(answers: impl IntoIterator<Item = Option<&'a str>>) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
                options: Vec::new(),
            }
        }
    }

    impl Menu for FakeMenu {
        fn choose(
            &mut self,
            _: &str,
            _: &str,
            options: &[String],
            _: u8,
        ) -> Result<Option<String>> {
            self.options.push(options.to_vec());
            Ok(self.answers.pop_front().flatten())
        }

        fn filter(&mut self, _: &str, _: &str, _: &[String], _: &str) -> Result<Option<String>> {
            Ok(None)
        }

        fn input(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: u16,
            _: InputIndent,
        ) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn layouts() -> Vec<TabLayout> {
        let template = Config::test_default().tab_layouts.remove(0);
        ["first", "second", "third"]
            .into_iter()
            .map(|name| {
                let mut layout = template.clone();
                layout.name = name.to_owned();
                layout
            })
            .collect()
    }

    fn config_with(layouts: Vec<TabLayout>, default: &str) -> Config {
        let mut config = Config::test_default();
        config.tab_layouts = layouts;
        config.default_tab_layout = default.to_owned();
        config
    }

    fn pinned_layout(name: &str, tab_label: &str) -> TabLayout {
        TabLayout {
            name: name.to_owned(),
            label: None,
            icon: None,
            tab_label: Some(tab_label.to_owned()),
            panes: vec![LayoutPane {
                name: "agent".to_owned(),
                label: None,
                icon: None,
                pane_type: PaneType::Agent,
                agent: Some("codex".to_owned()),
                option_name: None,
                command: None,
                direction: None,
                ratio: None,
                split_from: None,
                env: BTreeMap::new(),
            }],
        }
    }

    #[test]
    fn default_layout_is_listed_first_and_blank_is_always_last() {
        let config = config_with(layouts(), "second");
        let mut menu = FakeMenu::new([None]);

        choose_layout(&config, &mut menu).unwrap();

        assert_eq!(
            menu.options,
            [["second", "first", "third", &blank_tab_layout().menu_label()]]
        );
    }

    #[test]
    fn a_config_that_declares_no_layouts_still_offers_a_blank_tab() {
        // A config file with no `tab_layouts` section loads the shipping defaults, which
        // declare no blank layout. The menu adds one regardless.
        let ordered = ordered_layouts(&Config::test_default()).unwrap();

        assert_eq!(
            ordered.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["agentic-coding", BLANK_LAYOUT_NAME]
        );
    }

    #[test]
    fn a_configured_blank_layout_replaces_the_built_in_one_and_keeps_the_last_slot() {
        let mut layouts = layouts();
        let mut mine = layouts[0].clone();
        mine.name = BLANK_LAYOUT_NAME.to_owned();
        mine.label = Some("My Blank".to_owned());
        // Declared first in config; the menu still sorts it last.
        layouts.insert(0, mine);
        let config = config_with(layouts, "first");
        let mut menu = FakeMenu::new([None]);

        choose_layout(&config, &mut menu).unwrap();

        assert_eq!(menu.options, [["first", "second", "third", "My Blank"]]);
    }

    #[test]
    fn options_use_the_rendered_menu_label() {
        let mut layouts = layouts();
        layouts[0].label = Some("First Layout".to_owned());
        layouts[0].icon = Some("A".to_owned());
        let config = config_with(layouts, "second");
        let mut menu = FakeMenu::new([None]);

        choose_layout(&config, &mut menu).unwrap();

        assert_eq!(menu.options[0][1], "A  First Layout");
    }

    #[test]
    fn one_configured_layout_still_draws_the_menu_because_blank_joins_it() {
        let config = config_with(vec![layouts().remove(0)], "first");
        let mut menu = FakeMenu::new([Some("first")]);

        let selected = choose_layout(&config, &mut menu).unwrap().unwrap();

        assert_eq!(selected.name, "first");
        assert_eq!(menu.options, [["first", &blank_tab_layout().menu_label()]]);
    }

    #[test]
    fn none_cancels_layout_selection() {
        let config = config_with(layouts(), "first");
        let mut menu = FakeMenu::new([None]);

        assert!(choose_layout(&config, &mut menu).unwrap().is_none());
    }

    #[test]
    fn empty_selection_cancels_layout_selection() {
        let config = config_with(layouts(), "first");
        let mut menu = FakeMenu::new([Some("")]);

        assert!(choose_layout(&config, &mut menu).unwrap().is_none());
    }

    #[test]
    fn rendered_selection_and_its_padded_form_resolve_the_same_layout() {
        let config = config_with(layouts(), "first");
        let label = config.tab_layouts[2].menu_label();
        for answer in [label.clone(), format!("   {label}")] {
            let mut menu = FakeMenu::new([Some(answer.as_str())]);
            let selected = choose_layout(&config, &mut menu).unwrap().unwrap();
            assert_eq!(selected.name, "third");
        }
    }

    #[test]
    fn cancelling_before_popup_issues_no_client_calls() {
        let config = config_with(layouts(), "first");
        let client = FakeClient::default();
        let mut menu = FakeMenu::new([None]);

        // No env guard: cancelling returns before popup_with_layout reads the invoking
        // pane, so the no-call property has to hold whatever the ambient env says.
        let outcome = new_with(&client, &config, &mut menu).unwrap();

        assert_eq!(outcome, Outcome::Cancelled);
        assert!(client.calls.borrow().is_empty());
    }

    #[test]
    fn selected_layout_builds_its_tab() {
        // popup_with_layout adopts the invoking pane's cwd, which it finds through these
        // two variables. A real Herdr session sets them, and the pane they name is not
        // this test's, so they are cleared for the call and restored afterwards. The lock
        // keeps a parallel test from observing the gap.
        let _guard = crate::state::env_lock();
        let saved = ["HERDR_ACTIVE_PANE_ID", "HERDR_PLUGIN_CONTEXT_JSON"]
            .map(|name| (name, std::env::var_os(name)));
        for (name, _) in &saved {
            std::env::remove_var(name);
        }
        let layouts = vec![
            pinned_layout("first", "First tab"),
            pinned_layout("second", "Second tab"),
        ];
        let config = config_with(layouts, "first");
        let selected = config.tab_layouts[1].menu_label();
        let mut menu = FakeMenu::new([Some(selected.as_str())]);
        let client = FakeClient::default();
        client.queue_response(
            "tab.create",
            json!({
                "type": "tab_created",
                "root_pane": {"pane_id": "p1"},
                "tab": {"tab_id": "t1"},
            }),
        );

        let outcome = new_with(&client, &config, &mut menu);
        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }

        assert_eq!(outcome.unwrap(), Outcome::Done);
        let calls = client.calls.borrow();
        let create = calls
            .iter()
            .find(|(method, _)| method == "tab.create")
            .expect("tab.create call");
        assert_eq!(create.1["label"], "Second tab");
    }
}
