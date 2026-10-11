pub mod bundle;
pub mod claude;

use std::path::Path;

use bundle::{MARKETPLACE, PLUGIN_ID};
use claude::{Marketplace, Plugin, Step, same_path};

use crate::daemon::control::is_older;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub marketplaces: Vec<Marketplace>,
    pub plugins: Vec<Plugin>,
}

impl Found {
    pub fn marketplace(&self) -> Option<&Marketplace> {
        self.marketplaces.iter().find(|m| m.name == MARKETPLACE)
    }

    pub fn plugin(&self) -> Option<&Plugin> {
        self.plugins
            .iter()
            .find(|p| p.id == PLUGIN_ID && p.scope == "user")
    }
}

pub fn install_steps(found: &Found, dir: &Path, version: &str) -> Vec<Step> {
    let ours = found
        .marketplace()
        .and_then(|m| m.path.as_deref())
        .is_some_and(|path| same_path(Path::new(path), dir));
    let mut steps = Vec::new();
    if !ours {
        steps.push(Step::AddMarketplace(dir.to_path_buf()));
    }
    match found.plugin() {
        None => steps.push(Step::Install),
        Some(plugin) => {
            if is_older(&plugin.version, version) {
                steps.push(Step::Update {
                    from: plugin.version.clone(),
                });
            }
            if !plugin.enabled {
                steps.push(Step::Enable);
            }
        }
    }
    steps
}

pub fn uninstall_steps(found: &Found) -> Vec<Step> {
    let mut steps = Vec::new();
    if found.plugin().is_some() {
        steps.push(Step::Uninstall);
    }
    if found.marketplace().is_some() {
        steps.push(Step::RemoveMarketplace);
    }
    steps
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn ours(dir: &Path) -> Marketplace {
        Marketplace {
            name: MARKETPLACE.into(),
            source: "directory".into(),
            path: Some(dir.display().to_string()),
            repo: None,
            url: None,
        }
    }

    fn plus(version: &str, enabled: bool) -> Plugin {
        Plugin {
            id: PLUGIN_ID.into(),
            version: version.into(),
            scope: "user".into(),
            enabled,
            folder: None,
            folder_version: None,
        }
    }

    #[test]
    fn a_fresh_install_adds_the_marketplace_then_the_mod() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        assert_eq!(
            install_steps(&Found::default(), &dir, "0.1.0"),
            [Step::AddMarketplace(dir.clone()), Step::Install]
        );
    }

    #[test]
    fn an_installed_mod_of_this_version_needs_nothing() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("0.1.0", true)],
        };
        assert_eq!(install_steps(&found, &dir, "0.1.0"), []);
    }

    #[test]
    fn another_version_is_updated_and_a_disabled_mod_enabled() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("0.0.9", false)],
        };
        assert_eq!(
            install_steps(&found, &dir, "0.1.0"),
            [
                Step::Update {
                    from: "0.0.9".into()
                },
                Step::Enable
            ]
        );
    }

    #[test]
    fn a_newer_installed_mod_is_never_updated_down() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("999.0.0", false)],
        };
        assert_eq!(install_steps(&found, &dir, "0.1.0"), [Step::Enable]);
    }

    #[test]
    fn a_marketplace_of_that_name_elsewhere_is_pointed_at_this_folder() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let github = Marketplace {
            source: "github".into(),
            path: None,
            repo: Some("ada/claude-plus".into()),
            ..ours(&dir)
        };
        let found = Found {
            marketplaces: vec![github],
            plugins: vec![plus("0.1.0", true)],
        };
        assert_eq!(
            install_steps(&found, &dir, "0.1.0"),
            [Step::AddMarketplace(dir.clone())]
        );
    }

    #[test]
    fn a_project_scoped_copy_or_another_plugin_is_not_ours() {
        let dir = PathBuf::from("/m");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![
                Plugin {
                    scope: "project".into(),
                    ..plus("0.1.0", true)
                },
                Plugin {
                    id: "plus@elsewhere".into(),
                    ..plus("0.1.0", true)
                },
            ],
        };
        assert_eq!(install_steps(&found, &dir, "0.1.0"), [Step::Install]);
        assert_eq!(uninstall_steps(&found), [Step::RemoveMarketplace]);
    }

    #[test]
    fn uninstalling_removes_the_mod_before_its_marketplace() {
        let dir = PathBuf::from("/m");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("0.1.0", true)],
        };
        assert_eq!(
            uninstall_steps(&found),
            [Step::Uninstall, Step::RemoveMarketplace]
        );
        assert_eq!(uninstall_steps(&Found::default()), []);
    }
}
