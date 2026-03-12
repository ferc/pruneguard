use crate::config::OxgraphConfig;

/// Merge semantics: extends merge left-to-right, last wins.
pub(crate) trait Merge {
    fn merge_from(&mut self, base: &Self);
}

impl Merge for OxgraphConfig {
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

        // frameworks: last wins
        if self.frameworks.is_none() {
            self.frameworks.clone_from(&base.frameworks);
        }

        // overrides: append
        if !base.overrides.is_empty() {
            let mut merged = base.overrides.clone();
            merged.extend(self.overrides.clone());
            self.overrides = merged;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    #[test]
    fn test_ignore_patterns_append() {
        let mut child =
            OxgraphConfig { ignore_patterns: vec!["child/**".to_string()], ..Default::default() };
        let base =
            OxgraphConfig { ignore_patterns: vec!["base/**".to_string()], ..Default::default() };
        child.merge_from(&base);
        assert_eq!(child.ignore_patterns, vec!["base/**", "child/**"]);
    }

    #[test]
    fn test_workspaces_last_wins() {
        let mut child = OxgraphConfig::default();
        let base = OxgraphConfig {
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
        let mut child = OxgraphConfig {
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
        let base = OxgraphConfig {
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
