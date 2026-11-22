use std::borrow::Cow;
use std::cell::RefCell;
use std::io;
use std::process::{Command, Output};

mod observer;
mod protocol;

// These slots undergo one tmux format expansion. A final semicolon also
// separates commands in native argv before format expansion takes place.
pub(super) fn literal_format(value: &str) -> Cow<'_, str> {
    if !value.contains('#') && !value.ends_with(';') {
        return Cow::Borrowed(value);
    }
    let mut escaped = value.replace('#', "#{a:35}");
    if escaped.ends_with(';') {
        escaped.pop();
        escaped.push_str("#{a:59}");
    }
    Cow::Owned(escaped)
}

struct Monitor {
    profile: String,
    client: Option<protocol::Client>,
}

thread_local! {
    static MONITOR: RefCell<Option<Monitor>> = const { RefCell::new(None) };
}

pub(super) fn with_monitor<T>(profile: &str, run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            MONITOR.with(|slot| slot.borrow_mut().take());
        }
    }
    MONITOR.with(|slot| {
        assert!(slot.borrow().is_none(), "Nested tmux notification monitor");
        slot.replace(Some(Monitor {
            profile: profile.to_owned(),
            client: None,
        }));
    });
    let _reset = Reset;
    run()
}

pub(super) fn ensure_connected(owner_option: &str, owner: &str) -> anyhow::Result<bool> {
    MONITOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        let monitor = slot.as_mut().expect("monitor scope");
        if monitor.client.is_none() {
            monitor.client = observer::connect(&monitor.profile, owner_option, owner)?;
        }
        Ok(monitor.client.is_some())
    })
}

pub(crate) fn output(args: &[&str]) -> io::Result<Output> {
    run(None, args)
}

pub(crate) fn profile_output(profile: &str, args: &[&str]) -> io::Result<Output> {
    run(Some(profile), args)
}

fn run(profile: Option<&str>, args: &[&str]) -> io::Result<Output> {
    MONITOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(monitor) = slot.as_mut() else {
            return native_command(profile, args)?.output();
        };
        if profile.is_some_and(|p| p != monitor.profile) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tmux monitor profile mismatch",
            ));
        }
        let client = monitor.client.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "tmux monitor disconnected")
        })?;
        let result = client.run(args);
        if matches!(&result, Err(error) if error.kind() != io::ErrorKind::InvalidInput) {
            monitor.client.take();
        }
        result
    })
}

fn native_command(profile: Option<&str>, args: &[&str]) -> io::Result<Command> {
    let mut command = profile.map_or_else(super::tmux_command, super::profile_command);
    if profile.is_some() {
        command.arg("-u");
    }
    protocol::native_args(&mut command, args)?;
    Ok(command)
}
