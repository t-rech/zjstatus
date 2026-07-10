use lazy_static::lazy_static;
use regex::Regex;
use std::collections::BTreeMap;

#[cfg(all(not(feature = "bench"), not(test)))]
use zellij_tile::shim::run_command;

use crate::render::{FormattedPart, formatted_parts_from_string_cached};

use super::command::commandline_parser;
use super::widget::Widget;

lazy_static! {
    static ref PIPE_REGEX: Regex = Regex::new("_[a-zA-Z0-9]+$").unwrap();
}

#[derive(Clone, Debug, PartialEq)]
enum RenderMode {
    Static,
    Dynamic,
    Raw,
}

pub struct PipeWidget {
    config: BTreeMap<String, PipeConfig>,
    zj_conf: BTreeMap<String, String>,
}

#[derive(Clone)]
struct PipeConfig {
    format: Vec<FormattedPart>,
    render_mode: RenderMode,
    click_action: String,
    tab_scoped: bool,
}

impl PipeWidget {
    pub fn new(config: &BTreeMap<String, String>) -> Self {
        Self {
            config: parse_config(config),
            zj_conf: config.clone(),
        }
    }
}

impl Widget for PipeWidget {
    fn process(&self, name: &str, state: &crate::config::ZellijState) -> String {
        let pipe_config = match self.config.get(name) {
            Some(pc) => pc,
            None => {
                tracing::debug!("pipe no name {name}");
                return "".to_owned();
            }
        };

        let raw_result = match state.pipe_results.get(name) {
            Some(pr) => pr,
            None => {
                tracing::debug!("pipe no content {name}");
                return "".to_owned();
            }
        };

        // tab-scoped pipes carry one record per tab, separated by the ASCII
        // unit separator (0x1f): "<0-based tab position>|<content>". Each
        // plugin instance renders only the record of the tab it lives in.
        let pipe_result = &match pipe_config.tab_scoped {
            true => select_tab_record(raw_result, state),
            false => raw_result.to_owned(),
        };

        if pipe_config.tab_scoped && pipe_result.is_empty() {
            return "".to_owned();
        }

        let content = pipe_config
            .format
            .iter()
            .map(|f| {
                let mut content = f.content.clone();

                if content.contains("{output}") {
                    content = content.replace(
                        "{output}",
                        pipe_result.strip_suffix('\n').unwrap_or(pipe_result),
                    )
                }

                (f, content)
            })
            .fold("".to_owned(), |acc, (f, content)| {
                if pipe_config.render_mode == RenderMode::Static {
                    return format!("{acc}{}", f.format_string(&content));
                }

                format!("{acc}{}", content)
            });

        match pipe_config.render_mode {
            RenderMode::Static => content,
            RenderMode::Dynamic => render_dynamic_formatted_content(&content, &self.zj_conf),
            RenderMode::Raw => pipe_result.to_owned(),
        }
    }

    fn process_click(&self, name: &str, _state: &crate::config::ZellijState, _pos: usize) {
        let pipe_config = match self.config.get(name) {
            Some(pc) => pc,
            None => {
                return;
            }
        };

        if pipe_config.click_action.is_empty() {
            return;
        }

        let command = commandline_parser(&pipe_config.click_action);

        // fire-and-forget: click action results are of no use and must not be
        // processed — see the RunCommandResult handler
        let mut context: BTreeMap<String, String> = BTreeMap::new();
        context.insert("fire_and_forget".to_owned(), "true".to_owned());

        tracing::debug!("Running pipe click command {:?} {:?}", command, context);

        #[cfg(all(not(feature = "bench"), not(test)))]
        run_command(
            &command.iter().map(|x| x.as_str()).collect::<Vec<&str>>(),
            context,
        );

        #[cfg(any(feature = "bench", test))]
        let _ = (command, context);
    }
}

fn render_dynamic_formatted_content(content: &str, config: &BTreeMap<String, String>) -> String {
    formatted_parts_from_string_cached(content, config)
        .iter()
        .map(|fp| fp.format_string(&fp.content))
        .collect::<Vec<String>>()
        .join("")
}

/// Picks the record addressed to the tab this plugin instance lives in from a
/// tab-scoped payload. Records are separated by the ASCII unit separator
/// (0x1f) and formatted as "<0-based tab position>|<content>". Returns an
/// empty string when the own pane cannot be located or no record matches.
fn select_tab_record(payload: &str, state: &crate::config::ZellijState) -> String {
    let own_id = match state.plugin_pane_id {
        Some(id) => id,
        None => return "".to_owned(),
    };

    let own_tab = state.panes.panes.iter().find_map(|(pos, panes)| {
        panes
            .iter()
            .any(|p| p.is_plugin && p.id == own_id)
            .then_some(*pos)
    });

    let own_tab = match own_tab {
        Some(pos) => pos,
        None => return "".to_owned(),
    };

    for record in payload.split('\u{1f}') {
        if let Some((pos, content)) = record.split_once('|') {
            if pos.parse::<usize>() == Ok(own_tab) {
                return content.to_owned();
            }
        }
    }

    "".to_owned()
}

fn parse_config(zj_conf: &BTreeMap<String, String>) -> BTreeMap<String, PipeConfig> {
    let mut keys: Vec<String> = zj_conf
        .keys()
        .filter(|k| k.starts_with("pipe_"))
        .cloned()
        .collect();
    keys.sort();

    let mut config: BTreeMap<String, PipeConfig> = BTreeMap::new();

    for key in keys {
        let pipe_name = PIPE_REGEX.replace(&key, "").to_string();
        let mut pipe_conf = PipeConfig {
            format: vec![],
            render_mode: RenderMode::Static,
            click_action: "".to_owned(),
            tab_scoped: false,
        };

        if let Some(existing_conf) = config.get(pipe_name.as_str()) {
            pipe_conf = existing_conf.clone();
        }

        if key.ends_with("format") {
            pipe_conf.format =
                FormattedPart::multiple_from_format_string(zj_conf.get(&key).unwrap(), zj_conf);
        }

        if key.ends_with("clickaction") {
            pipe_conf.click_action = zj_conf.get(&key).unwrap().to_owned();
        }

        if key.ends_with("tabscoped") {
            pipe_conf.tab_scoped = matches!(zj_conf.get(&key).map(|v| v.as_str()), Some("true"));
        }

        if key.ends_with("rendermode") {
            pipe_conf.render_mode = match zj_conf.get(&key) {
                Some(mode) => match mode.as_str() {
                    "static" => RenderMode::Static,
                    "dynamic" => RenderMode::Dynamic,
                    "raw" => RenderMode::Raw,
                    _ => RenderMode::Static,
                },
                None => RenderMode::Static,
            };
        }

        config.insert(pipe_name, pipe_conf);
    }
    config
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::ZellijState;
    use zellij_tile::prelude::{PaneInfo, PaneManifest};

    fn state_with_plugin_in_tab(tab: usize, plugin_id: u32) -> ZellijState {
        let plugin_pane = PaneInfo {
            id: plugin_id,
            is_plugin: true,
            ..PaneInfo::default()
        };
        let mut panes = std::collections::HashMap::new();
        panes.insert(tab, vec![plugin_pane]);

        ZellijState {
            plugin_pane_id: Some(plugin_id),
            panes: PaneManifest { panes },
            ..ZellijState::default()
        }
    }

    #[test]
    fn test_select_tab_record() {
        let payload = "0|zero\u{1f}1|one\u{1f}2|two";

        let state = state_with_plugin_in_tab(1, 7);
        assert_eq!(select_tab_record(payload, &state), "one");

        let state = state_with_plugin_in_tab(2, 7);
        assert_eq!(select_tab_record(payload, &state), "two");

        // no record for the own tab
        let state = state_with_plugin_in_tab(5, 7);
        assert_eq!(select_tab_record(payload, &state), "");

        // own pane unknown
        let mut state = state_with_plugin_in_tab(1, 7);
        state.plugin_pane_id = None;
        assert_eq!(select_tab_record(payload, &state), "");

        // content may contain further pipes.. only the first one splits
        let state = state_with_plugin_in_tab(0, 7);
        assert_eq!(select_tab_record("0|a|b", &state), "a|b");
    }
}
