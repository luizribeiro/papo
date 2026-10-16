use std::{
    env, fs, io,
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{certificate, directory::TemporaryDirectory};

pub(crate) struct Configuration {
    pub(crate) plaintext: std::net::SocketAddr,
    pub(crate) tls: std::net::SocketAddr,
    pub(crate) websocket: std::net::SocketAddr,
    pub(crate) secure_websocket: std::net::SocketAddr,
    pub(crate) services: std::net::SocketAddr,
}

impl Configuration {
    fn with_paths(directory: &TemporaryDirectory, unreal: &str, modules: &str) -> io::Result<Self> {
        let address = || TcpListener::bind(("127.0.0.1", 0))?.local_addr();
        let config = Self {
            plaintext: address()?,
            tls: address()?,
            websocket: address()?,
            secure_websocket: address()?,
            services: address()?,
        };
        config.write(directory.path(), unreal, modules)?;
        Ok(config)
    }

    fn write(&self, directory: &Path, unreal: &str, modules: &str) -> io::Result<()> {
        let path = |name| directory.join(name).display().to_string();
        fs::create_dir_all(path("tmp/tmp"))?;
        fs::write(path("empty.txt"), "\n")?;
        fs::write(path("services.db"), "")?;
        fs::write(
            path("unrealircd.conf"),
            format!(
                r##"@define $NO_DEFAULT_GEOIP "1"
include "{unreal}/modules.default.conf";
include "{unreal}/operclass.default.conf";
include "{unreal}/snomasks.default.conf";
loadmodule "webserver";
loadmodule "websocket";
loadmodule "cloak_none";
me {{ name "irc.papo.test"; info "papo test server"; sid "001"; }}
admin {{ "papo"; "papo@example.invalid"; }}
class clients {{ pingfreq 90; maxclients 1000; sendq 1M; recvq 8000; }}
class servers {{ pingfreq 60; connfreq 5; maxclients 10; sendq 20M; }}
allow {{ mask 127.0.0.1; class clients; maxperip 100; }}
listen {{ ip 127.0.0.1; port {plain}; }}
listen {{ ip 127.0.0.1; port {tls}; options {{ tls; }} tls-options {{ certificate "{cert}"; key "{key}"; }}; }}
listen {{ ip 127.0.0.1; port {ws}; options {{ websocket {{ type text; }} }}; }}
listen {{ ip 127.0.0.1; port {wss}; options {{ tls; websocket {{ type text; }} }} tls-options {{ certificate "{cert}"; key "{key}"; }}; }}
listen {{ ip 127.0.0.1; port {services}; options {{ serversonly; }} }}
link services.papo.test {{ incoming {{ mask 127.0.0.1; }} password "papo-link"; class servers; }}
ulines {{ services.papo.test; }}
set {{
 sasl-server services.papo.test;
 tls {{ certificate "{cert}"; key "{key}"; sts-policy {{ port {tls}; duration 3600; }} }}
 modes-on-join "+ntH 100:1d";
 kline-address "papo@example.invalid"; network-name "papo"; default-server "irc.papo.test"; help-channel "#help";
 options {{ identd-check; }}
 plaintext-policy {{ user allow; oper warn; server warn; }}
 anti-flood {{ everyone {{ connect-flood 255:10; }} unknown-users {{ nick-flood 255:10; lag-penalty 1; lag-penalty-bytes 10000; }} }}
}}
tld {{ mask *; motd "{empty}"; botmotd "{empty}"; rules "{empty}"; }}
files {{ pidfile "{pid}"; tunefile "{tune}"; }}
"##,
                plain = self.plaintext.port(),
                tls = self.tls.port(),
                ws = self.websocket.port(),
                wss = self.secure_websocket.port(),
                services = self.services.port(),
                cert = path("server.pem"),
                key = path("server.key"),
                empty = path("empty.txt"),
                pid = path("unrealircd.pid"),
                tune = path("ircd.tune"),
            ),
        )?;
        fs::write(
            path("atheme.conf"),
            format!(
                r#"loadmodule "{modules}/protocol/unreal4";
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
uplink "irc.papo.test" {{ host = "127.0.0.1"; port = {port}; send_password = "papo-link"; receive_password = "papo-link"; }};
saslserv {{ nick = "SaslServ"; }};
"#,
                port = self.services.port()
            ),
        )
    }

    fn command(name: &str, directory: &TemporaryDirectory) -> io::Result<Command> {
        let path = |file| directory.path().join(file);
        let mut command = Command::new(required_env(name)?);
        if name == "UNREALIRCD_BIN" {
            command
                .args(["-t", "-F", "-f"])
                .arg(path("unrealircd.conf"));
        } else {
            command.arg("-n");
            for (flag, file) in [
                ("-c", "atheme.conf"),
                ("-l", "atheme.log"),
                ("-p", "atheme.pid"),
            ] {
                command.arg(flag).arg(path(file));
            }
            command.arg("-D").arg(directory.path());
        }
        command.current_dir(directory.path());
        Ok(command)
    }
}

pub(crate) struct TestServer {
    children: Vec<Child>,
    directory: TemporaryDirectory,
    config: Configuration,
}

impl TestServer {
    pub(crate) fn start() -> io::Result<Self> {
        for _ in 0..4 {
            if let Ok(server) = Self::start_once() {
                return Ok(server);
            }
        }
        Self::start_once()
    }

    fn start_once() -> io::Result<Self> {
        let directory = TemporaryDirectory::create()?;
        certificate::generate(directory.path())?;
        let config = Configuration::with_paths(
            &directory,
            &required_env("UNREALIRCD_CONFIG")?,
            &required_env("ATHEME_MODULES")?,
        )?;
        let unreal = Configuration::command("UNREALIRCD_BIN", &directory)?
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut server = Self {
            children: vec![unreal],
            directory,
            config,
        };
        server.wait_for(false)?;
        let atheme = Configuration::command("ATHEME_BIN", &server.directory)?
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        server.children.push(atheme);
        server.wait_for(true)?;
        Ok(server)
    }

    fn wait_for(&mut self, services: bool) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for child in &mut self.children {
                if let Some(status) = child.try_wait()? {
                    return Err(io::Error::other(format!(
                        "test server exited with {status}"
                    )));
                }
            }
            if if services {
                self.services_linked()
            } else {
                [
                    self.config.plaintext,
                    self.config.tls,
                    self.config.websocket,
                    self.config.secure_websocket,
                ]
                .iter()
                .all(|address| TcpStream::connect(address).is_ok())
            } {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "test server did not start",
        ))
    }

    fn services_linked(&self) -> bool {
        fs::read_to_string(self.directory.path().join("atheme.log"))
            .is_ok_and(|log| log.contains("finished synching with uplink"))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        for child in self.children.iter_mut().rev() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn required_env(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| io::Error::other(format!("{name} is unset")))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs};

    use crate::directory::TemporaryDirectory;

    use super::Configuration;

    #[test]
    fn writes_isolated_unrealircd_and_atheme_configs() {
        let directory = TemporaryDirectory::create().unwrap();
        let config =
            Configuration::with_paths(&directory, "/unreal/conf", "/atheme/modules").unwrap();
        let addresses = [
            config.plaintext,
            config.tls,
            config.websocket,
            config.secure_websocket,
            config.services,
        ];
        assert_eq!(addresses.iter().collect::<HashSet<_>>().len(), 5);
        assert!(addresses.iter().all(|address| address.ip().is_loopback()));
        let unreal = fs::read_to_string(directory.path().join("unrealircd.conf")).unwrap();
        assert!(unreal.contains("/unreal/conf/modules.default.conf"));
        assert!(unreal.contains(&format!("port {};", config.tls.port())));
        assert!(unreal.contains("modes-on-join \"+ntH 100:1d\""));
        let atheme = fs::read_to_string(directory.path().join("atheme.conf")).unwrap();
        assert!(atheme.contains("/atheme/modules/protocol/unreal4"));
        assert!(atheme.contains(&format!("port = {}", config.services.port())));
    }

    #[test]
    fn starts_unrealircd_with_linked_atheme() {
        super::TestServer::start().unwrap();
    }
}
