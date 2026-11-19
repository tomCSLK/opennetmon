//! Files the server creates itself when they are missing (config `generate_missing`, set in the container
//! image): the key file and a self-signed certificate. The `.deb` and the installer create both during the
//! installation instead. Existing files are never touched; a certificate without its key (or the other way
//! round) is an error, so a half-copied pair is not silently replaced.

use std::path::Path;

use crate::config::Config;
use crate::crypto::SecretBox;
use crate::tls;

/// What [`create_missing`] created.
#[derive(Debug, Default)]
pub struct Created {
    pub key_file: bool,
    pub certificate: Option<tls::Generated>,
}

/// Creates the key file and the certificate of `config` where they do not exist yet.
pub fn create_missing(config: &Config) -> Result<Created, String> {
    let mut created = Created::default();
    if !exists(&config.secret_key_file)? {
        SecretBox::generate_key_file(&config.secret_key_file).map_err(|e| e.to_string())?;
        created.key_file = true;
    }
    if let Some((cert, key)) = config.tls_files() {
        match (exists(cert)?, exists(key)?) {
            (true, true) => {}
            (false, false) => {
                // Names found on the system (e.g. a host name with "_") are left out.
                let names: Vec<String> = tls::certificate_names(&[])
                    .into_iter()
                    .filter(|name| tls::is_valid_name(name))
                    .collect();
                created.certificate =
                    Some(tls::generate(cert, key, &names).map_err(|e| e.to_string())?);
            }
            (cert_exists, _) => {
                let (there, missing) = if cert_exists {
                    (cert, key)
                } else {
                    (key, cert)
                };
                return Err(format!(
                    "{} exists but {} does not; restore the missing file or remove the other one to get a new certificate",
                    there.display(),
                    missing.display()
                ));
            }
        }
    }
    Ok(created)
}

/// Whether `path` exists; an error (e.g. no permission on the directory) is reported, not taken as "no".
fn exists(path: &Path) -> Result<bool, String> {
    path.try_exists()
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dir: &Path, tls: bool) -> Config {
        Config {
            secret_key_file: dir.join("conf/secret.key"),
            tls_cert_file: tls.then(|| dir.join("conf/tls-cert.pem")),
            tls_key_file: tls.then(|| dir.join("conf/tls-key.pem")),
            ..Config::default()
        }
    }

    #[test]
    fn missing_files_are_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path(), true);
        let first = create_missing(&config).unwrap();
        assert!(first.key_file);
        let cert = first.certificate.expect("certificate");
        assert!(cert.names.iter().any(|n| n == "localhost"));
        SecretBox::from_key_file(&config.secret_key_file).unwrap();
        let (cert_file, key_file) = config.tls_files().unwrap();
        tls::load(cert_file, key_file).unwrap();

        let key_before = std::fs::read(&config.secret_key_file).unwrap();
        let second = create_missing(&config).unwrap();
        assert!(!second.key_file && second.certificate.is_none());
        assert_eq!(std::fs::read(&config.secret_key_file).unwrap(), key_before);
    }

    #[test]
    fn without_tls_only_the_key_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_missing(&config(dir.path(), false)).unwrap();
        assert!(created.key_file && created.certificate.is_none());
    }

    #[test]
    fn half_a_certificate_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path(), true);
        create_missing(&config).unwrap();
        let (cert_file, key_file) = config.tls_files().unwrap();
        std::fs::remove_file(key_file).unwrap();
        let err = create_missing(&config).unwrap_err();
        assert!(err.contains("tls-key.pem"), "{err}");
        assert!(cert_file.exists() && !key_file.exists());
    }
}
