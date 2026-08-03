use anyhow::{Context, Result};

use super::menu::{strip_pad, GumMenu, Menu};
use super::{agent, FlowResult, Outcome};
use crate::config::{Config, TabLayout};
use crate::herdr::HerdrClient;

const TITLE: &str = "\u{eb03}  Tab Layout";
const SUBTITLE: &str = "Choose a layout.";
const HEIGHT: u8 = 8;

fn choose_layout<'a>(config: &'a Config, menu: &mut impl Menu) -> Result<Option<&'a TabLayout>> {
    if config.tab_layouts.len() == 1 {
        return Ok(config.tab_layouts.first());
    }

    let default = config
        .layout(&config.default_tab_layout)
        .expect("validated at load");
    let options = std::iter::once(default)
        .chain(
            config
                .tab_layouts
                .iter()
                .filter(|layout| layout.name != config.default_tab_layout),
        )
        .map(TabLayout::menu_label)
        .collect::<Vec<_>>();
    let Some(selection) = menu.choose(TITLE, SUBTITLE, &options, HEIGHT)? else {
        return Ok(None);
    };
    let selection = strip_pad(&selection);
    if selection.is_empty() {
        return Ok(None);
    }

    Ok(Some(
        config
            .tab_layouts
            .iter()
            .find(|layout| layout.menu_label() == selection)
            .expect("validated at load"),
    ))
}

pub fn new(client: &dyn HerdrClient) -> FlowResult {
    let config = Config::load().context("failed to load config")?;
    let (cols, lines) = agent::popup_viewport();
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
    agent::popup_with_layout(client, config, layout, false)
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

    fn without_invoking_pane_env<T>(run: impl FnOnce() -> T) -> T {
        let saved = ["HERDR_ACTIVE_PANE_ID", "HERDR_PLUGIN_CONTEXT_JSON"]
            .map(|name| (name, std::env::var_os(name)));
        for (name, _) in &saved {
            std::env::remove_var(name);
        }
        let result = run();
        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        result
    }

    #[test]
    fn default_layout_is_listed_first_without_sorting_the_rest() {
        let config = config_with(layouts(), "second");
        let mut menu = FakeMenu::new([None]);

        choose_layout(&config, &mut menu).unwrap();

        assert_eq!(menu.options, [["second", "first", "third"]]);
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
    fn one_layout_skips_the_menu() {
        let config = config_with(vec![layouts().remove(0)], "first");
        let mut menu = FakeMenu::new([]);

        let selected = choose_layout(&config, &mut menu).unwrap().unwrap();

        assert_eq!(selected.name, "first");
        assert!(menu.options.is_empty());
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
        let _guard = crate::state::env_lock();
        let config = config_with(layouts(), "first");
        let client = FakeClient::default();
        let mut menu = FakeMenu::new([None]);

        let outcome = without_invoking_pane_env(|| new_with(&client, &config, &mut menu)).unwrap();

        assert_eq!(outcome, Outcome::Cancelled);
        assert!(client.calls.borrow().is_empty());
    }

    #[test]
    fn selected_layout_builds_its_tab() {
        let _guard = crate::state::env_lock();
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

        let outcome = without_invoking_pane_env(|| new_with(&client, &config, &mut menu)).unwrap();

        assert_eq!(outcome, Outcome::Done);
        let calls = client.calls.borrow();
        let create = calls
            .iter()
            .find(|(method, _)| method == "tab.create")
            .expect("tab.create call");
        assert_eq!(create.1["label"], "Second tab");
        assert_ne!(create.1["label"], "First tab");
    }
}
