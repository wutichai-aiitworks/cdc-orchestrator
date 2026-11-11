// Append to the module holding AppMode. Same cases as templates/tests/mode.test.ts.
#[cfg(test)]
mod app_mode_tests {
    use super::*;

    #[test]
    fn parses_the_three_modes_in_common_spellings() {
        for v in ["Production", "PRODUCTION", "prod"] {
            assert_eq!(AppMode::parse(v), Some(AppMode::Production), "{v}");
        }
        for v in ["MAINTENANCE", "Maintenance", " maintainance ", "maint"] {
            assert_eq!(AppMode::parse(v), Some(AppMode::Maintenance), "{v}");
        }
        for v in ["ENHANCED", "Enhanced", "enhance", "Enhancement"] {
            assert_eq!(AppMode::parse(v), Some(AppMode::Enhanced), "{v}");
        }
        for v in ["", "dev", "uat", "sit"] {
            assert_eq!(AppMode::parse(v), None, "{v}");
        }
    }

    #[test]
    fn resolves_mode_then_retired_env_then_production() {
        assert_eq!(AppMode::resolve(Some("Enhanced"), Some("prod")), AppMode::Enhanced);
        assert_eq!(AppMode::resolve(None, Some("dev")), AppMode::Enhanced);
        assert_eq!(AppMode::resolve(None, Some("prod")), AppMode::Production);
        assert_eq!(AppMode::resolve(Some("bogus"), None), AppMode::Production);
        assert_eq!(AppMode::resolve(None, None), AppMode::Production);
    }

    #[test]
    fn only_production_is_hardened_by_default() {
        assert!(AppMode::Production.hardened_by_default());
        assert!(!AppMode::Maintenance.hardened_by_default());
        assert!(!AppMode::Enhanced.hardened_by_default());
    }
}
