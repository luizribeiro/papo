use std::{env, fs, io};

use crate::{atheme, directory::TemporaryDirectory};

pub(super) struct Configuration {
    pub(super) endpoints: [std::net::SocketAddr; 4],
    pub(super) services: std::net::SocketAddr,
}

impl Configuration {
    pub(super) fn new(directory: &TemporaryDirectory) -> io::Result<Self> {
        Self::with_paths(
            directory,
            &required_env("INSPIRCD_MODULES")?,
            &required_env("ATHEME_MODULES")?,
        )
    }

    fn with_paths(
        directory: &TemporaryDirectory,
        modules: &str,
        atheme_modules: &str,
    ) -> io::Result<Self> {
        let [plain, tls, websocket, secure_websocket, services] =
            crate::ports::allocate_loopback()?;
        let config = Self {
            endpoints: [plain, tls, websocket, secure_websocket],
            services,
        };
        config.write(directory, modules, atheme_modules)?;
        Ok(config)
    }

    fn write(
        &self,
        directory: &TemporaryDirectory,
        modules: &str,
        atheme_modules: &str,
    ) -> io::Result<()> {
        let [plain, tls, websocket, secure_websocket] = self.endpoints;
        let path = |name| directory.path().join(name).display().to_string();
        fs::write(
            path("inspircd.conf"),
            format!(
                r#"<path module="{modules}">
<server name="irc.papo.test" description="papo test server" network="papo" id="001">
<admin name="papo" email="papo@example.invalid">
<bind address="127.0.0.1" port="{plain}" type="clients">
<bind address="127.0.0.1" port="{tls}" type="clients" sslprofile="Clients">
<bind address="127.0.0.1" port="{websocket}" type="clients" hook="websocket">
<bind address="127.0.0.1" port="{secure_websocket}" type="clients" hook="websocket" sslprofile="Clients">
<bind address="127.0.0.1" port="{services}" type="servers">
<connect allow="*" resolvehostnames="no" recvq="40960" localmax="1000" globalmax="1000">
<class name="Servers" pingfreq="60" connectfreq="5" sendq="20M">
<link name="services.papo.test" ipaddr="127.0.0.1" port="{services}" allowmask="*" recvpass="papo-link" sendpass="papo-link" class="Servers">
<uline server="services.papo.test"><security announceinvites="none">
<sslprofile name="Clients" provider="gnutls" certfile="{cert}" keyfile="{key}">
<sasl target="services.papo.test" requiressl="no"><websocket defaultmode="text" nativeping="no"><wsorigin allow="*">
<module name="account"><module name="botmode"><module name="cap"><module name="chghost">
<module name="ircv3"><module name="ircv3_accounttag"><module name="ircv3_batch"><module name="ircv3_capnotify"><module name="ircv3_chghost"><module name="ircv3_ctctags"><module name="ircv3_echomessage"><module name="ircv3_invitenotify"><module name="ircv3_labeledresponse"><module name="ircv3_msgid"><module name="ircv3_servertime">
<module name="monitor"><module name="multiprefix"><module name="sasl"><module name="services"><module name="setname"><module name="sha1"><module name="spanningtree"><module name="ssl_gnutls"><module name="uhnames"><module name="websocket">
"#,
                plain = plain.port(),
                tls = tls.port(),
                websocket = websocket.port(),
                secure_websocket = secure_websocket.port(),
                services = self.services.port(),
                cert = path("server.pem"),
                key = path("server.key"),
            ),
        )?;
        atheme::write_config(
            directory.path(),
            atheme_modules,
            "inspircd",
            "irc.papo.test",
            self.services.port(),
        )
    }
}

fn required_env(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| io::Error::other(format!("{name} is unset")))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::directory::TemporaryDirectory;

    #[test]
    fn writes_ircv3_tls_websocket_and_atheme_configuration() {
        let directory = TemporaryDirectory::create().unwrap();
        let config =
            super::Configuration::with_paths(&directory, "/inspircd/modules", "/atheme/modules")
                .unwrap();
        let ircd = fs::read_to_string(directory.path().join("inspircd.conf")).unwrap();
        assert!(ircd.contains("<module name=\"ircv3\">"));
        assert!(ircd.contains("<module name=\"ssl_gnutls\">"));
        assert!(ircd.contains(&format!("port=\"{}\"", config.endpoints[3].port())));
        let atheme = fs::read_to_string(directory.path().join("atheme.conf")).unwrap();
        assert!(atheme.contains("/atheme/modules/protocol/inspircd"));
    }
}
