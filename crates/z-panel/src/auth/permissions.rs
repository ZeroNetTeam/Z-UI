//! What an admin may do: permissions are `<section>.read` or
//! `<section>.write`, and writing includes reading. Managing admins and
//! roles is not a permission anyone can be given: only a sudo admin does it.

use axum::http::Method;

/// A part of the panel permissions are given for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Users,
    Groups,
    Templates,
    Inbounds,
    Hosts,
    Certificates,
    Core,
    Monitor,
    Logs,
    Settings,
    Audit,
    Api,
}

impl Section {
    pub const ALL: [Section; 12] = [
        Section::Users,
        Section::Groups,
        Section::Templates,
        Section::Inbounds,
        Section::Hosts,
        Section::Certificates,
        Section::Core,
        Section::Monitor,
        Section::Logs,
        Section::Settings,
        Section::Audit,
        Section::Api,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::Users => "users",
            Section::Groups => "groups",
            Section::Templates => "templates",
            Section::Inbounds => "inbounds",
            Section::Hosts => "hosts",
            Section::Certificates => "certificates",
            Section::Core => "core",
            Section::Monitor => "monitor",
            Section::Logs => "logs",
            Section::Settings => "settings",
            Section::Audit => "audit",
            Section::Api => "api",
        }
    }

    pub fn parse(name: &str) -> Option<Section> {
        Self::ALL.into_iter().find(|section| section.name() == name)
    }

    /// Whether there is anything to change there; the audit log is only read.
    pub fn writable(self) -> bool {
        self != Section::Audit
    }

    fn index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|section| *section == self)
            .expect("every section is listed") as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Read,
    Write,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Read => "read",
            Level::Write => "write",
        }
    }

    /// What a request needs: looking is reading, anything else writes.
    pub fn of_method(method: &Method) -> Level {
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
            Level::Read
        } else {
            Level::Write
        }
    }
}

/// A set of permissions, two bits a section. Writing sets reading too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Permissions(u32);

impl Permissions {
    pub const NONE: Permissions = Permissions(0);

    fn bit(section: Section, level: Level) -> u32 {
        1 << (section.index() * 2 + u32::from(level == Level::Write))
    }

    /// Every permission there is.
    pub fn all() -> Self {
        Section::ALL.into_iter().fold(Self::NONE, |set, section| {
            set.with(
                section,
                if section.writable() {
                    Level::Write
                } else {
                    Level::Read
                },
            )
        })
    }

    /// This set plus `level` on `section`.
    pub fn with(self, section: Section, level: Level) -> Self {
        let mut bits = self.0 | Self::bit(section, Level::Read);
        if level == Level::Write {
            bits |= Self::bit(section, Level::Write);
        }
        Self(bits)
    }

    pub fn can(self, section: Section, level: Level) -> bool {
        self.0 & Self::bit(section, level) != 0
    }

    /// What both sets allow.
    pub fn and(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Read a list of `section.level` names, refusing any it does not know.
    pub fn parse<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let mut set = Self::NONE;
        for name in names {
            let name = name.trim();
            let parsed = name.split_once('.').and_then(|(section, level)| {
                let section = Section::parse(section)?;
                match level {
                    "read" => Some((section, Level::Read)),
                    "write" if section.writable() => Some((section, Level::Write)),
                    _ => None,
                }
            });
            let (section, level) = parsed.ok_or_else(|| format!("no such permission {name:?}"))?;
            set = set.with(section, level);
        }
        Ok(set)
    }

    /// Read what the database keeps: `*` for everything, else a
    /// comma-separated list. Entries it does not know are dropped: a
    /// permission can only be lost by that, never gained.
    pub fn from_stored(text: &str) -> Self {
        let text = text.trim();
        if text == "*" {
            return Self::all();
        }
        text.split(',')
            .filter_map(|name| Self::parse([name]).ok())
            .fold(Self::NONE, |set, one| Self(set.0 | one.0))
    }

    /// The text the database keeps: the highest level of each section.
    pub fn to_stored(self) -> String {
        self.highest()
            .into_iter()
            .map(|(section, level)| format!("{}.{}", section.name(), level.name()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Every permission in the set, each level named: what a client checks.
    pub fn names(self) -> Vec<String> {
        let mut names = Vec::new();
        for section in Section::ALL {
            for level in [Level::Read, Level::Write] {
                if self.can(section, level) {
                    names.push(format!("{}.{}", section.name(), level.name()));
                }
            }
        }
        names
    }

    fn highest(self) -> Vec<(Section, Level)> {
        Section::ALL
            .into_iter()
            .filter_map(|section| {
                if self.can(section, Level::Write) {
                    Some((section, Level::Write))
                } else if self.can(section, Level::Read) {
                    Some((section, Level::Read))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_includes_reading_and_nothing_else_follows() {
        let set = Permissions::NONE
            .with(Section::Users, Level::Write)
            .with(Section::Groups, Level::Read);
        assert!(set.can(Section::Users, Level::Read) && set.can(Section::Users, Level::Write));
        assert!(set.can(Section::Groups, Level::Read) && !set.can(Section::Groups, Level::Write));
        assert!(!set.can(Section::Core, Level::Read));
        assert_eq!(set.names(), ["users.read", "users.write", "groups.read"]);
        assert_eq!(set.to_stored(), "users.write,groups.read");
    }

    #[test]
    fn a_stored_set_comes_back_the_same_and_unknown_names_cost_a_permission() {
        let all = Permissions::all();
        assert_eq!(Permissions::from_stored(&all.to_stored()), all);
        assert_eq!(Permissions::from_stored("*"), all);
        assert!(all.can(Section::Audit, Level::Read) && !all.can(Section::Audit, Level::Write));
        assert_eq!(
            Permissions::from_stored("users.read, teleport.write,core.nope,,core.write"),
            Permissions::NONE
                .with(Section::Users, Level::Read)
                .with(Section::Core, Level::Write)
        );
        assert_eq!(Permissions::from_stored(""), Permissions::NONE);
    }

    #[test]
    fn a_list_from_a_client_is_checked_strictly() {
        let set = Permissions::parse(["users.write", " hosts.read "]).unwrap();
        assert!(set.can(Section::Hosts, Level::Read) && !set.can(Section::Hosts, Level::Write));
        for bad in ["users", "users.admin", "nothing.read", "audit.write", ""] {
            assert!(Permissions::parse([bad]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn two_sets_meet_in_what_both_allow() {
        let admin = Permissions::from_stored("users.write,core.read");
        let key = Permissions::from_stored("users.read,core.write,hosts.write");
        let both = admin.and(key);
        assert!(both.can(Section::Users, Level::Read) && !both.can(Section::Users, Level::Write));
        assert!(both.can(Section::Core, Level::Read) && !both.can(Section::Core, Level::Write));
        assert!(!both.can(Section::Hosts, Level::Read));
    }

    #[test]
    fn requests_read_or_write_by_their_method() {
        assert_eq!(Level::of_method(&Method::GET), Level::Read);
        assert_eq!(Level::of_method(&Method::HEAD), Level::Read);
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            assert_eq!(Level::of_method(&method), Level::Write);
        }
    }

    #[test]
    fn the_default_roles_name_only_permissions_that_exist() {
        for (name, permissions, _) in z_db::DEFAULT_ROLES {
            let listed = permissions.split(',').count();
            let parsed = Permissions::from_stored(permissions).to_stored();
            assert_eq!(parsed.split(',').count(), listed, "{name}: {permissions}");
            assert!(Permissions::parse(permissions.split(',')).is_ok(), "{name}");
        }
    }
}
