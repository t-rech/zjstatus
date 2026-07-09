use std::collections::BTreeMap;

#[cfg(all(not(feature = "bench"), not(test)))]
use zellij_tile::shim::run_command;

use crate::{
    config::ZellijState,
    widgets::{command::commandline_parser, widget::Widget},
};

pub struct SessionWidget {
    click_action: Vec<String>,
}

impl SessionWidget {
    pub fn new(config: &BTreeMap<String, String>) -> Self {
        Self {
            click_action: match config.get("session_click_action") {
                Some(action) => commandline_parser(action),
                None => Vec::new(),
            },
        }
    }
}

impl Widget for SessionWidget {
    fn process(&self, _name: &str, state: &ZellijState) -> String {
        match &state.mode.session_name {
            Some(name) => name.to_owned(),
            None => "".to_owned(),
        }
    }

    fn process_click(&self, _name: &str, _state: &ZellijState, _pos: usize) {
        if self.click_action.is_empty() {
            return;
        }

        // Fire-and-forget: the marker makes the RunCommandResult handler drop
        // the result before touching any state. Click actions have no use for
        // the result, and on zellij 0.44.x (WASMI) processing command results
        // can crash the plugin (#247) — commands spawned outside the render
        // path with their results dropped are safe.
        let mut context = BTreeMap::new();
        context.insert("fire_and_forget".to_owned(), "true".to_owned());

        tracing::debug!("Running session click command {:?}", self.click_action);

        #[cfg(all(not(feature = "bench"), not(test)))]
        run_command(
            &self
                .click_action
                .iter()
                .map(|x| x.as_str())
                .collect::<Vec<&str>>(),
            context,
        );

        #[cfg(any(feature = "bench", test))]
        let _ = context;
    }
}
