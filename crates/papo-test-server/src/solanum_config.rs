use std::{env, fs, io};

use crate::{atheme, directory::TemporaryDirectory};

pub(super) struct Configuration {
    pub(super) plaintext: std::net::SocketAddr,
    pub(super) tls: std::net::SocketAddr,
    pub(super) services: std::net::SocketAddr,
}

impl Configuration {
    pub(super) fn new(directory: &TemporaryDirectory) -> io::Result<Self> {
        Self::with_paths(
            directory,
            &required_env("SOLANUM_MODULES")?,
            &required_env("ATHEME_MODULES")?,
        )
    }

    fn with_paths(
        directory: &TemporaryDirectory,
        modules: &str,
        atheme_modules: &str,
    ) -> io::Result<Self> {
        let [plaintext, tls, services] = crate::ports::allocate_loopback()?;
        let config = Self {
            plaintext,
            tls,
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
        let path = |name| directory.path().join(name).display().to_string();
        fs::write(
            path("solanum.conf"),
            format!(
                r#"loadmodule "extensions/extb_account";
loadmodule "extensions/invite_notify";
loadmodule "extensions/m_echotags";
loadmodule "extensions/tag_message_id";
loadmodule "extensions/tag_typing";
serverinfo {{ name = "irc.papo.test"; sid = "001"; description = "papo test server"; network_name = "papo"; ssl_cert = "{cert}"; ssl_private_key = "{key}"; ssld_count = 1; default_max_clients = 1000; nicklen = 30; }};
admin {{ name = "papo"; description = "papo test server"; email = "papo@example.invalid"; }};
class "users" {{ ping_time = 2 minutes; number_per_ident = 100; number_per_ip = 100; number_per_ip_global = 100; max_number = 1000; sendq = 1 megabyte; }};
class "server" {{ ping_time = 5 minutes; connectfreq = 5 minutes; max_number = 10; max_autoconn = 0; sendq = 4 megabytes; }};
listen {{ defer_accept = no; host = "127.0.0.1"; port = {plain}; port = {services}; sslport = {tls}; }};
auth {{ user = "*@*"; flags = exceed_limit; class = "users"; }};
privset "papo_oper" {{ privs = oper:general, oper:kline; }};
operator "{oper}" {{ user = "*@127.0.0.1"; password = "{oper_password}"; flags = ~encrypted; privset = "papo_oper"; }};
connect "services.papo.test" {{ host = "127.0.0.1"; port = 0; send_password = "papo-link"; accept_password = "papo-link"; class = "server"; flags = topicburst; }};
service {{ name = "services.papo.test"; }};
exempt {{ ip = "127.0.0.1"; }};
channel {{ use_invex = yes; use_except = yes; no_create_on_split = no; no_join_on_split = no; disable_local_channels = no; displayed_usercount = 0; autochanmodes = "+nt"; invite_notify_notice = no; }};
general {{ throttle_count = 100; pace_wait_simple = 0 second; pace_wait = 0 second; connect_timeout = 1 second; sasl_service = "SaslServ"; max_monitor = 100; post_registration_delay = 0 seconds; }};
modules {{ path = "{modules}"; path = "{modules}/autoload"; }};
"#,
                cert = path("server.pem"),
                key = path("server.key"),
                plain = self.plaintext.port(),
                services = self.services.port(),
                tls = self.tls.port(),
                oper = crate::OPER_NAME,
                oper_password = crate::OPER_PASSWORD,
            ),
        )?;
        atheme::write_config(
            directory.path(),
            atheme_modules,
            "solanum",
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
    fn writes_tls_ircv3_and_atheme_configuration() {
        let directory = TemporaryDirectory::create().unwrap();
        let config =
            super::Configuration::with_paths(&directory, "/solanum/modules", "/atheme/modules")
                .unwrap();
        let ircd = fs::read_to_string(directory.path().join("solanum.conf")).unwrap();
        assert!(ircd.contains("extensions/tag_message_id"));
        assert!(ircd.contains(&format!("sslport = {}", config.tls.port())));
        assert!(ircd.contains("connect_timeout = 1 second"));
        assert!(ircd.contains("privs = oper:general, oper:kline"));
        assert!(ircd.contains("password = \"papo-oper-password\"; flags = ~encrypted"));
        let atheme = fs::read_to_string(directory.path().join("atheme.conf")).unwrap();
        assert!(atheme.contains("/atheme/modules/protocol/solanum"));
    }
}
