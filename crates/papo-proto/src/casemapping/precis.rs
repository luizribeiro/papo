use std::borrow::Cow;

use precis_profiles::{UsernameCaseMapped, precis_core::profile::PrecisFastInvocation as _};

use super::Casemapping;

pub(super) fn fold(name: &str) -> Cow<'_, str> {
    UsernameCaseMapped::enforce(name).unwrap_or_else(|_| {
        // Ergo rejects these names, but map keys still need one deterministic fold.
        Casemapping::Ascii.fold(name)
    })
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::Casemapping;

    #[test]
    fn folds_unicode_nicks_with_ergo_precis_rules() {
        assert_eq!("rfc7613".parse(), Ok(Casemapping::Precis));
        assert_eq!(Casemapping::Precis.to_string(), "rfc7613");
        assert_eq!(Casemapping::Precis.fold("Ünïcode"), "ünïcode");
        assert_eq!(Casemapping::Precis.fold("İstanbul"), "i\u{307}stanbul");
        assert_eq!(Casemapping::Precis.nick("ÜNÏCODE"), "ünïcode");
        assert_ne!(Casemapping::Precis.nick("İstanbul"), "istanbul");
        assert!(matches!(
            Casemapping::Precis.fold("ünïcode"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn folds_precis_rejections_as_ascii() {
        assert_eq!(Casemapping::Precis.fold("Bad Nick"), "bad nick");
        assert_eq!(Casemapping::Precis.fold("Bad\u{7}Nick"), "bad\u{7}nick");
    }
}
