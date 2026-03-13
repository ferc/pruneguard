use crate::config::{FrameworksConfig, PruneguardConfig};

/// Merge semantics: extends merge left-to-right, last wins.
pub(crate) trait Merge {
    fn merge_from(&mut self, base: &Self);
}

impl Merge for PruneguardConfig {
    fn merge_from(&mut self, base: &Self) {
        // ignore_patterns: append
        if !base.ignore_patterns.is_empty() {
            let mut merged = base.ignore_patterns.clone();
            merged.extend(self.ignore_patterns.clone());
            self.ignore_patterns = merged;
        }

        // workspaces: last wins (self takes precedence over base)
        if self.workspaces.is_none() {
            self.workspaces.clone_from(&base.workspaces);
        }

        // rules: concatenate arrays
        if let Some(base_rules) = &base.rules {
            if let Some(self_rules) = &mut self.rules {
                let mut forbidden = base_rules.forbidden.clone();
                forbidden.extend(self_rules.forbidden.clone());
                self_rules.forbidden = forbidden;

                let mut required = base_rules.required.clone();
                required.extend(self_rules.required.clone());
                self_rules.required = required;

                let mut allow = base_rules.allow.clone();
                allow.extend(self_rules.allow.clone());
                self_rules.allow = allow;
            } else {
                self.rules = Some(base_rules.clone());
            }
        }

        // ownership: last wins
        if self.ownership.is_none() {
            self.ownership.clone_from(&base.ownership);
        }

        // frameworks: per-field merge (each toggle uses last-wins)
        if let Some(base_fw) = &base.frameworks {
            if let Some(self_fw) = &mut self.frameworks {
                self_fw.merge_from(base_fw);
            } else {
                self.frameworks = Some(base_fw.clone());
            }
        }

        // overrides: append
        if !base.overrides.is_empty() {
            let mut merged = base.overrides.clone();
            merged.extend(self.overrides.clone());
            self.overrides = merged;
        }
    }
}

/// Merge individual framework toggles: self takes precedence, base fills gaps.
impl Merge for FrameworksConfig {
    fn merge_from(&mut self, base: &Self) {
        macro_rules! merge_toggle {
            ($($field:ident),+ $(,)?) => {
                $(
                    if self.$field.is_none() {
                        self.$field = base.$field;
                    }
                )+
            };
        }
        merge_toggle!(
            next, vite, vitest, jest, storybook, nuxt, astro, sveltekit, remix, angular, nx,
            turborepo, playwright, cypress, vitepress, docusaurus,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    #[test]
    fn test_ignore_patterns_append() {
        let mut child = PruneguardConfig {
            ignore_patterns: vec!["child/**".to_string()],
            ..Default::default()
        };
        let base =
            PruneguardConfig { ignore_patterns: vec!["base/**".to_string()], ..Default::default() };
        child.merge_from(&base);
        assert_eq!(child.ignore_patterns, vec!["base/**", "child/**"]);
    }

    #[test]
    fn test_workspaces_last_wins() {
        let mut child = PruneguardConfig::default();
        let base = PruneguardConfig {
            workspaces: Some(WorkspacesConfig {
                roots: vec!["packages/*".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        child.merge_from(&base);
        assert!(child.workspaces.is_some());
        assert_eq!(child.workspaces.unwrap().roots, vec!["packages/*"]);
    }

    #[test]
    fn test_rules_concatenate() {
        let mut child = PruneguardConfig {
            rules: Some(RulesConfig {
                forbidden: vec![Rule {
                    name: "child-rule".to_string(),
                    severity: AnalysisSeverity::Error,
                    comment: None,
                    from: None,
                    to: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let base = PruneguardConfig {
            rules: Some(RulesConfig {
                forbidden: vec![Rule {
                    name: "base-rule".to_string(),
                    severity: AnalysisSeverity::Warn,
                    comment: None,
                    from: None,
                    to: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        child.merge_from(&base);
        let rules = child.rules.unwrap();
        assert_eq!(rules.forbidden.len(), 2);
        assert_eq!(rules.forbidden[0].name, "base-rule");
        assert_eq!(rules.forbidden[1].name, "child-rule");
    }
}
