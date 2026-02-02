#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserContext {
    vars: Vec<(String, String)>,
}

impl UserContext {
    pub fn new<K, V, I>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: vars
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    pub fn apply(&self) -> EnvGuard {
        let mut previous = Vec::with_capacity(self.vars.len());

        for (key, value) in &self.vars {
            let current = std::env::var_os(key);
            previous.push((key.clone(), current));
            // SAFETY: This guard is intended to manage scoped process env changes.
            unsafe {
                std::env::set_var(key, value);
            }
        }

        EnvGuard { previous }
    }
}

pub struct EnvGuard {
    previous: Vec<(String, Option<std::ffi::OsString>)>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => {
                    // SAFETY: Restore prior environment value captured when applying.
                    unsafe {
                        std::env::set_var(&key, value);
                    }
                }
                None => {
                    // SAFETY: Remove only keys captured as absent when applying.
                    unsafe {
                        std::env::remove_var(&key);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UserContext;

    #[test]
    fn user_context_applies_and_restores_env() {
        unsafe {
            std::env::set_var("GLOWBERRY_TEST_ENV_RESTORE", "initial");
        }

        let context = UserContext::new([("GLOWBERRY_TEST_ENV_RESTORE", "applied")]);
        let _guard = context.apply();

        assert_eq!(
            std::env::var("GLOWBERRY_TEST_ENV_RESTORE").as_deref(),
            Ok("applied")
        );

        drop(_guard);

        assert_eq!(
            std::env::var("GLOWBERRY_TEST_ENV_RESTORE").as_deref(),
            Ok("initial")
        );
    }

    #[test]
    fn user_context_restores_duplicate_keys_in_reverse_order() {
        unsafe {
            std::env::set_var("GLOWBERRY_TEST_ENV_DUPLICATE", "initial");
        }

        let context = UserContext::new([
            ("GLOWBERRY_TEST_ENV_DUPLICATE", "first"),
            ("GLOWBERRY_TEST_ENV_DUPLICATE", "second"),
        ]);
        let _guard = context.apply();

        assert_eq!(
            std::env::var("GLOWBERRY_TEST_ENV_DUPLICATE").as_deref(),
            Ok("second")
        );

        drop(_guard);

        assert_eq!(
            std::env::var("GLOWBERRY_TEST_ENV_DUPLICATE").as_deref(),
            Ok("initial")
        );
    }
}
