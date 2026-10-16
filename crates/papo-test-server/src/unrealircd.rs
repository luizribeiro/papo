use std::{
    env, fs, io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{atheme, certificate, directory::TemporaryDirectory};

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
 tls {{ certificate "{cert}"; key "{key}"; }}
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
        atheme::write_config(
            directory,
            modules,
            "unreal4",
            "irc.papo.test",
            self.services.port(),
        )
    }

    fn command(name: &str, directory: &TemporaryDirectory) -> io::Result<Command> {
        let path = |file| directory.path().join(file);
        let mut command = Command::new(required_env(name)?);
        if name == "UNREALIRCD_BIN" {
            command
                .args(["-t", "-F", "-f"])
                .arg(path("unrealircd.conf"));
        }
        command.current_dir(directory.path());
        Ok(command)
    }
}

#[doc(hidden)]
/// An UnrealIRCd process linked to Atheme for the target runner.
pub struct TestServer {
    children: Vec<Child>,
    directory: TemporaryDirectory,
    config: Configuration,
}

impl TestServer {
    /// Starts `UnrealIRCd` and Atheme and waits for their services link.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration, process startup, or readiness fails.
    pub fn start() -> io::Result<Self> {
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
        let atheme = atheme::command(&server.directory)?
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        server.children.push(atheme);
        server.wait_for(true)?;
        Ok(server)
    }

    fn wait_for(&mut self, services: bool) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        if !services {
            let endpoints = [
                self.config.plaintext,
                self.config.tls,
                self.config.websocket,
                self.config.secure_websocket,
            ];
            return crate::server::wait_until_ready_until(
                "UnrealIRCd",
                endpoints,
                deadline,
                || {
                    for child in &mut self.children {
                        if let Some(status) = child.try_wait()? {
                            return Ok(Some(status));
                        }
                    }
                    Ok(None)
                },
            );
        }
        while Instant::now() < deadline {
            for child in &mut self.children {
                if let Some(status) = child.try_wait()? {
                    return Err(io::Error::other(format!(
                        "test server exited with {status}"
                    )));
                }
            }
            if self.services_linked() {
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
        atheme::linked(self.directory.path())
    }

    /// Returns the plaintext, TLS, WebSocket, and secure WebSocket listeners.
    #[must_use]
    pub const fn endpoints(&self) -> [std::net::SocketAddr; 4] {
        [
            self.config.plaintext,
            self.config.tls,
            self.config.websocket,
            self.config.secure_websocket,
        ]
    }

    /// Returns the generated CA certificate path.
    #[must_use]
    pub fn ca_pem_path(&self) -> PathBuf {
        self.directory.path().join("ca.pem")
    }

    /// Returns the `UnrealIRCd` process identifier.
    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.children[0].id()
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
        assert!(!unreal.contains("sts-policy"));
    }

    #[test]
    fn registers_an_account_through_linked_atheme() {
        let server = super::TestServer::start().unwrap();
        let name = crate::unique("atheme-account");
        crate::account::register_atheme(server.config.plaintext, &name, "q7Hx-2mZp_9vLcT4wRfB")
            .unwrap();
        assert!(
            crate::account::register_atheme(server.config.plaintext, &name, "q7Hx-2mZp_9vLcT4wRfB")
                .is_err()
        );
    }
}
