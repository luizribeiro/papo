use std::{
    env, fs, io,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use super::{directory::TemporaryDirectory, process::ServerChild, startup::StartupBudget};

pub(crate) fn write_config(
    directory: &Path,
    modules: &str,
    protocol: &str,
    uplink: &str,
    port: u16,
) -> io::Result<()> {
    let chanserv = if protocol == "solanum" {
        format!(
            "loadmodule \"{modules}/chanserv/main\";\nloadmodule \"{modules}/chanserv/register\";\nloadmodule \"{modules}/chanserv/set_guard\";"
        )
    } else {
        String::new()
    };
    let chanserv_config = if protocol == "solanum" {
        r#"chanserv { nick = "ChanServ"; user = "ChanServ"; host = "services.int"; real = "Channel Services"; maxchans = 5; };"#
    } else {
        ""
    };
    fs::write(directory.join("services.db"), "")?;
    fs::write(
        directory.join("atheme.conf"),
        format!(
            r#"loadmodule "{modules}/protocol/{protocol}";
loadmodule "{modules}/backend/opensex";
loadmodule "{modules}/crypto/pbkdf2v2";
loadmodule "{modules}/crypto/pbkdf2";
loadmodule "{modules}/nickserv/main";
loadmodule "{modules}/nickserv/cert";
loadmodule "{modules}/nickserv/register";
loadmodule "{modules}/nickserv/verify";
{chanserv}
loadmodule "{modules}/saslserv/main";
loadmodule "{modules}/saslserv/authcookie";
loadmodule "{modules}/saslserv/external";
loadmodule "{modules}/saslserv/plain";
serverinfo {{ name = "services.papo.test"; desc = "papo test services"; numeric = "00A"; recontime = 1; netname = "papo"; adminname = "papo"; adminemail = "papo@example.invalid"; registeremail = "papo@example.invalid"; auth = none; casemapping = "ascii"; }};
general {{ commit_interval = 5; join_chans; }};
{chanserv_config}
uplink "{uplink}" {{ host = "127.0.0.1"; port = {port}; send_password = "papo-link"; receive_password = "papo-link"; }};
saslserv {{ nick = "SaslServ"; }};
"#,
        ),
    )
}

/// Starts Atheme and waits until it has synced with `ircd`, which already accepts clients.
pub(crate) fn link(
    directory: &TemporaryDirectory,
    ircd_name: &str,
    ircd: &mut ServerChild,
) -> io::Result<ServerChild> {
    let mut atheme = ServerChild::spawn(&mut command(directory)?)?;
    let budget = StartupBudget::new(Instant::now());
    wait_for_link(directory.path(), (ircd_name, ircd), &mut atheme, budget)?;
    Ok(atheme)
}

fn wait_for_link(
    directory: &Path,
    (ircd_name, ircd): (&str, &mut ServerChild),
    atheme: &mut ServerChild,
    mut budget: StartupBudget,
) -> io::Result<()> {
    loop {
        ircd.check(ircd_name)?;
        atheme.check("Atheme")?;
        if linked(directory) {
            return Ok(());
        }
        atheme.charge("Atheme", &mut budget)?;
        thread::sleep(Duration::from_millis(20));
    }
}

fn command(directory: &TemporaryDirectory) -> io::Result<Command> {
    let path = |file| directory.path().join(file);
    let mut command =
        Command::new(env::var("ATHEME_BIN").map_err(|_| io::Error::other("ATHEME_BIN is unset"))?);
    command.arg("-n");
    for (flag, file) in [
        ("-c", "atheme.conf"),
        ("-l", "atheme.log"),
        ("-p", "atheme.pid"),
    ] {
        command.arg(flag).arg(path(file));
    }
    command.arg("-D").arg(directory.path());
    command.current_dir(directory.path());
    Ok(command)
}

fn linked(directory: &Path) -> bool {
    fs::read_to_string(directory.join("atheme.log"))
        .is_ok_and(|log| log.contains("finished synching with uplink"))
}

#[cfg(test)]
mod tests {
    use std::{fs, io, process::Command};

    use super::super::directory::TemporaryDirectory;
    use crate::native::{process::ServerChild, startup::tests::settled_budget};

    const CONNECTED: &str = "[2026-11-17 04:40:38] uplink_connect(): connecting to first entry irc.papo.test[127.0.0.1]:33159.
[2026-11-17 04:40:38] irc_handle_connect(): connection to uplink established
";

    fn sleeping() -> ServerChild {
        ServerChild::spawn(Command::new("sleep").arg("60")).unwrap()
    }

    fn wait_with(log: &str, mut ircd: ServerChild) -> io::Result<()> {
        let directory = TemporaryDirectory::create().unwrap();
        fs::write(directory.path().join("atheme.log"), log).unwrap();
        let mut atheme = sleeping();
        let budget = settled_budget(&[atheme.id(), ircd.id()], false);
        super::wait_for_link(
            directory.path(),
            ("Solanum", &mut ircd),
            &mut atheme,
            budget,
        )
    }

    #[test]
    fn a_link_that_never_syncs_spends_the_startup_budget() {
        let error = wait_with(CONNECTED, sleeping()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let message = error.to_string();
        assert!(
            message.starts_with(
                "Atheme did not start: stayed idle before becoming ready (11.0 s idle"
            ),
            "{message}"
        );
    }

    #[test]
    fn a_synced_link_is_ready_whatever_the_budget_says() {
        let synced = "[2026-11-17 04:40:38] m_pong(): finished synching with uplink (0 ms)\n";

        wait_with(&format!("{CONNECTED}{synced}"), sleeping()).unwrap();
    }

    #[test]
    fn an_ircd_that_exits_ends_the_link_wait() {
        let exiting = ServerChild::spawn(Command::new("sh").args(["-c", "exit 1"])).unwrap();
        let stat = format!("/proc/{}/stat", exiting.id());
        while !fs::read_to_string(&stat).unwrap().contains(") Z ") {
            std::thread::yield_now();
        }

        let message = wait_with(CONNECTED, exiting).unwrap_err().to_string();

        assert!(
            message.starts_with("Solanum exited with exit status: 1"),
            "{message}"
        );
    }

    #[test]
    fn writes_protocol_and_uplink_configuration() {
        let directory = TemporaryDirectory::create().unwrap();
        super::write_config(
            directory.path(),
            "/atheme/modules",
            "unreal4",
            "irc.papo.test",
            16667,
        )
        .unwrap();
        let config = fs::read_to_string(directory.path().join("atheme.conf")).unwrap();
        assert!(config.contains("/atheme/modules/protocol/unreal4"));
        assert!(config.contains("recontime = 1"));
        assert!(!config.contains("/atheme/modules/chanserv/set_guard"));
        assert!(!config.contains("chanserv {"));
        assert!(config.contains("uplink \"irc.papo.test\""));
        assert!(config.contains("port = 16667"));
        assert!(directory.path().join("services.db").is_file());
    }
}
