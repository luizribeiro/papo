use std::{fs, io, net::TcpListener, path::Path};

use crate::directory::TemporaryDirectory;

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
        fs::write(path("empty.txt"), "\n")?;
        fs::write(path("services.db"), "")?;
        fs::write(
            path("unrealircd.conf"),
            format!(
                r#"@define NO_DEFAULT_GEOIP
include "{unreal}/modules.default.conf";
include "{unreal}/operclass.default.conf";
include "{unreal}/snomasks.default.conf";
loadmodule "webserver";
loadmodule "websocket";
me {{ name "irc.papo.test"; info "papo test server"; sid "001"; }}
admin {{ "papo"; "papo@example.invalid"; }}
class clients {{ pingfreq 90; maxclients 1000; sendq 1M; recvq 1M; }}
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
 modes-on-join "+ntH 100:1d";
 kline-address "papo@example.invalid"; network-name "papo"; default-server "irc.papo.test";
 cloak-keys {{ "papoTestKey1"; "papoTestKey2"; "papoTestKey3"; }}
 options {{ identd-check; }}
 plaintext-policy {{ user allow; oper warn; server warn; }}
 anti-flood {{ everyone {{ connect-flood 255:10; }} unknown-users {{ nick-flood 255:10; lag-penalty 1; lag-penalty-bytes 10000; }} }}
}}
tld {{ mask *; motd "{empty}"; botmotd "{empty}"; rules "{empty}"; }}
files {{ pidfile "{pid}"; tunefile "{tune}"; }}
"#,
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
}
