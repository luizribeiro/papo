use std::{env, fs, io, path::Path, process::Command};

use crate::directory::TemporaryDirectory;

pub(crate) fn write_config(
    directory: &Path,
    modules: &str,
    protocol: &str,
    uplink: &str,
    port: u16,
) -> io::Result<()> {
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
loadmodule "{modules}/saslserv/main";
loadmodule "{modules}/saslserv/authcookie";
loadmodule "{modules}/saslserv/external";
loadmodule "{modules}/saslserv/plain";
serverinfo {{ name = "services.papo.test"; desc = "papo test services"; numeric = "00A"; netname = "papo"; adminname = "papo"; adminemail = "papo@example.invalid"; registeremail = "papo@example.invalid"; auth = none; casemapping = "ascii"; }};
general {{ commit_interval = 5; }};
uplink "{uplink}" {{ host = "127.0.0.1"; port = {port}; send_password = "papo-link"; receive_password = "papo-link"; }};
saslserv {{ nick = "SaslServ"; }};
"#,
        ),
    )
}

pub(crate) fn command(directory: &TemporaryDirectory) -> io::Result<Command> {
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

pub(crate) fn linked(directory: &Path) -> bool {
    fs::read_to_string(directory.join("atheme.log"))
        .is_ok_and(|log| log.contains("finished synching with uplink"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::directory::TemporaryDirectory;

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
        assert!(config.contains("uplink \"irc.papo.test\""));
        assert!(config.contains("port = 16667"));
        assert!(directory.path().join("services.db").is_file());
    }
}
