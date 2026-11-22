use std::process::{Command, Output};

use anyhow::{ensure, Context, Result};
use uuid::Uuid;

use super::protocol::Client;
use crate::tmux::profile_command;

const BUSINESS: &str = "#{S:#{?#{m:aoe_*,#{session_name}},1,}}";
const GENERATION: &str = "#{pid}/#{start_time}";
const TOKEN: &str = "AOE_CONTROL_OBSERVER_TOKEN";
const MARK: &str = "@aoe_control_observer";
const BINDING: &str = "@aoe_observer_client_pane";

fn checked(output: Output, operation: &str) -> Result<String> {
    ensure!(
        output.status.success(),
        "{operation} ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    String::from_utf8(output.stdout).context("Invalid UTF-8 in tmux observer response")
}

fn eligible(profile: &str, option: &str, owner: &str) -> Result<Option<String>> {
    let format = format!("{GENERATION} #{{?{BUSINESS},1,0}}");
    let literal_option = super::literal_format(option);
    let output = profile_command(profile)
        .args([
            "-N",
            "-u",
            "display-message",
            "-p",
            &format,
            ";",
            "show-options",
            "-gqv",
            &literal_option,
        ])
        .output()
        .context("Starting tmux observer preflight")?;
    let response = checked(output, "Reading tmux observer preflight")?;
    let (header, value) = response
        .split_once('\n')
        .context("Missing observer preflight")?;
    let (server, active) = header
        .split_once(' ')
        .context("Missing observer activity")?;
    let (pid, start) = server.split_once('/').context("Missing tmux generation")?;
    ensure!(
        pid.parse::<u32>()? > 0 && start.parse::<u64>()? > 0,
        "Invalid tmux generation"
    );
    ensure!(matches!(active, "0" | "1"), "Invalid observer activity");
    Ok((active == "1" && value.strip_suffix('\n') == Some(owner)).then(|| server.to_owned()))
}

struct Observer {
    name: String,
    token: Uuid,
    server: String,
}

impl Observer {
    fn owned_empty(&self) -> String {
        let fields = format!(
            "{GENERATION}|#{{session_name}}|#{{{TOKEN}}}|#{{{MARK}}}|\
             #{{session_windows}}|#{{window_panes}}|#{{pane_pid}}|#{{pane_tty}}"
        );
        format!(
            "#{{==:{fields},{}|{}|{}|1|1|1|0|}}",
            self.server, self.name, self.token
        )
    }

    fn command(&self, profile: &str) -> Command {
        let guard = format!("#{{&&:#{{==:{GENERATION},{}}},{BUSINESS}}}", self.server);
        let name = &self.name;
        let token = self.token;
        let bootstrap = format!(
            "#{{==:#{{{TOKEN}}}|#{{session_windows}}|#{{window_panes}}|#{{pane_start_command}},\
             {token}|1|1|/bin/sleep 1}}"
        );
        let departing = format!("#{{==:#{{hook_client}}/#{{pane_id}},#{{{BINDING}}}}}");
        let clean = format!("#{{&&:{},{departing}}}", self.owned_empty());
        // tmux 3.4 supports a processless pane only in split-window. The short
        // bootstrap uses exec argv, bypassing both default-command and the shell.
        // Use destroy-unattached during setup only. The hook protects adopted panes
        // and also works when tmux has migrated a human client into the observer.
        let create = format!(
            "new-session -E -s {name} -e {TOKEN}={token} -f no-output,ignore-size /bin/sleep 1 ; \
             if-shell -F -t ={name}: '{bootstrap}' {{ \
             set-option -p -t ={name}: remain-on-exit off ; \
             set-option -t ={name}: destroy-unattached on ; \
             set-option -t ={name}: detach-on-destroy on ; \
             set-option -t ={name}: {MARK} 1 ; \
             run-shell -C -t ={name}: \
                 \"split-window -d -t #{{pane_id}} '' ; kill-pane -t #{{pane_id}}\" ; \
             set-option -F -t ={name}: {BINDING} '#{{client_name}}/#{{pane_id}}' ; \
             set-hook -a -t ={name}: client-detached {{ \
                 if-shell -F -t ={name}: '{clean}' {{ kill-session -t ={name} }} \
             }} ; set-option -t ={name}: destroy-unattached off }}"
        );
        let mut command = profile_command(profile);
        command.args(["-N", "-u", "-C", "if-shell", "-F", &guard, &create]);
        command
    }

    fn start(&self, profile: &str, option: &str, owner: &str) -> Result<Option<Client>> {
        let name = &self.name;
        let mut client = Client::start(self.command(profile))?;
        let format = format!(
            "{} #{{client_session}} #{{destroy-unattached}} #{{detach-on-destroy}} \
             #{{==:#{{{BINDING}}},#{{client_name}}/#{{pane_id}}}} #{{?{BUSINESS},1,0}}",
            self.owned_empty()
        );
        let output = client.run(&["display-message", "-p", &format])?;
        let state = checked(output, "Verifying tmux observer")?;
        let (state, active) = state
            .trim_end()
            .rsplit_once(' ')
            .context("Missing observer activity")?;
        ensure!(matches!(active, "0" | "1"), "Invalid observer activity");
        ensure!(
            state == format!("1 {name} off on 1"),
            "Observer identity, session options, or empty pane did not match for {name}"
        );
        let output = client.run(&["show-options", "-gqv", option])?;
        let published = checked(output, "Verifying global tmux monitor owner")?;
        Ok((active == "1" && published.strip_suffix('\n') == Some(owner)).then_some(client))
    }

    fn rollback(&self, profile: &str) -> Result<()> {
        let name = &self.name;
        let original_pane = format!("#{{?{BINDING},#{{m:*/#{{pane_id}},#{{{BINDING}}}}},1}}");
        let guard = format!("#{{&&:{},{original_pane}}}", self.owned_empty());
        let clean = format!("if-shell -F -t ={name}: '{guard}' {{ kill-session -t ={name} }}");
        let output = profile_command(profile)
            .args(["-N", "if-shell", "-F", &format!("#{{N/s:{name}}}"), &clean])
            .output()
            .context("Starting tmux observer rollback")?;
        checked(output, "Rolling back tmux observer")?;
        Ok(())
    }
}

pub(crate) fn connect(
    profile: &str,
    owner_option: &str,
    expected_owner: &str,
) -> Result<Option<Client>> {
    crate::session::tui_instances::validate_profile(profile)?;
    ensure!(
        owner_option
            .strip_prefix('@')
            .is_some_and(|key| !key.is_empty() && !key.chars().any(char::is_control)),
        "Invalid tmux monitor owner option"
    );
    ensure!(
        !expected_owner.is_empty() && !expected_owner.chars().any(char::is_control),
        "Invalid tmux monitor owner"
    );
    let Some(server) = eligible(profile, owner_option, expected_owner)? else {
        return Ok(None);
    };
    let observer = Observer {
        name: format!("aoe-observer-{}", Uuid::new_v4()),
        token: Uuid::new_v4(),
        server,
    };
    match observer.start(profile, owner_option, expected_owner) {
        Ok(client) => Ok(client),
        Err(error) => {
            if let Err(cleanup) = observer.rollback(profile) {
                tracing::warn!("Failed to roll back {}: {cleanup:#}", observer.name);
            }
            match eligible(profile, owner_option, expected_owner) {
                Ok(None) => Ok(None),
                Ok(Some(_)) => Err(error),
                Err(check) => {
                    Err(error.context(format!("Rechecking tmux observer eligibility: {check:#}")))
                }
            }
        }
    }
}
