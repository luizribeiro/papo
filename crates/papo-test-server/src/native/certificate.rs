use std::{
    io,
    path::Path,
    process::{Command, Stdio},
};

pub(crate) fn generate(directory: &Path) -> io::Result<()> {
    openssl(
        directory,
        &[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=papo-test-CA",
            "-keyout",
            "ca.key",
            "-out",
            "ca.pem",
            "-days",
            "2",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
        ],
    )?;
    openssl(
        directory,
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-keyout",
            "server.key",
            "-out",
            "server.csr",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
        ],
    )?;
    openssl(
        directory,
        &[
            "x509",
            "-req",
            "-in",
            "server.csr",
            "-CA",
            "ca.pem",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "server.pem",
            "-days",
            "2",
            "-copy_extensions",
            "copy",
        ],
    )
}

fn openssl(directory: &Path, args: &[&str]) -> io::Result<()> {
    let status = Command::new("openssl")
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("openssl failed with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs, process::Command};

    use super::generate;

    #[test]
    fn generates_a_server_certificate_signed_by_the_ca() {
        let directory = env::temp_dir().join(format!("papo-cert-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        generate(&directory).unwrap();
        let status = Command::new("openssl")
            .args(["verify", "-CAfile", "ca.pem", "server.pem"])
            .current_dir(&directory)
            .status()
            .unwrap();
        fs::remove_dir_all(directory).unwrap();
        assert!(status.success());
    }
}
