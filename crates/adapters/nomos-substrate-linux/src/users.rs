//! Accounts through the distribution's own tools ([substrate-contract.md],
//! Users on Linux; ADR 0013 §4): `useradd`, `usermod`, and `userdel`, each
//! run by a direct `execve` of its absolute path with a fixed argument
//! vector, a fixed environment, and no shell (AGENTS.md rule 5). The
//! database is read directly ([`crate::accounts`]); the tools own its
//! locking and its shadow files, and Nomos never writes them itself.
//!
//! [substrate-contract.md]: ../../../../docs/formal/substrate-contract.md

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nomos_core::condition::{AccountClass, UserCondition};
use nomos_core::effect::Receipt;
use nomos_core::observation::UserEvidence;
use nomos_core::resource::AccountName;

use crate::accounts::{Accounts, User};

/// The directory the tools are run from; nothing is looked up on a path.
const TOOLS: &str = "/usr/sbin";
/// The whole environment a tool runs with.
const PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// The evidence of `name` in `accounts`.
pub fn evidence(accounts: &Accounts, name: &AccountName) -> UserEvidence {
    match accounts.user(name.as_str()) {
        Some(u) => UserEvidence::Present {
            uid: u.uid,
            gid: u.gid,
            home: u.home.clone(),
            shell: u.shell.clone(),
        },
        None => UserEvidence::Absent,
    }
}

/// One run of a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The tool's absolute path.
    pub program: PathBuf,
    /// Its arguments, in order.
    pub args: Vec<OsString>,
}

impl Invocation {
    fn new(tool: &str, prefix: Option<&Path>) -> Self {
        let mut args = Vec::new();
        if let Some(root) = prefix {
            args.push("--prefix".into());
            args.push(root.as_os_str().to_owned());
        }
        Invocation {
            program: Path::new(TOOLS).join(tool),
            args,
        }
    }

    fn arg(mut self, a: &str) -> Self {
        self.args.push(a.into());
        self
    }

    fn name(mut self, name: &AccountName) -> Self {
        self.args.push("--".into());
        self.args.push(name.as_str().into());
        self
    }

    /// Runs the tool: `true` when it exits with status 0.
    pub fn run(&self) -> bool {
        Command::new(&self.program)
            .args(&self.args)
            .env_clear()
            .env("PATH", PATH)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

/// What the adapter does for `requirement` with the account `before`:
/// refuse, nothing, or run one tool (Users on Linux, Mutation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Refused before any effect.
    Refuse,
    /// Already as required.
    Nothing,
    /// Run this.
    Run(Invocation),
}

/// The command for `requirement`, given the database and what it holds for
/// `name`. `prefix` is the root when it is not `/`.
pub fn plan(
    accounts: &Accounts,
    name: &AccountName,
    requirement: &UserCondition,
    prefix: Option<&Path>,
) -> Plan {
    let before: Option<&User> = accounts.user(name.as_str());
    if before.is_some_and(|u| accounts.holders(u.uid) > 1) {
        return Plan::Refuse;
    }
    match (requirement, before) {
        (UserCondition::Absent, None) => Plan::Nothing,
        (UserCondition::Absent, Some(_)) => {
            Plan::Run(Invocation::new("userdel", prefix).name(name))
        }
        (UserCondition::Present { class, home, shell }, None) => {
            let mut run = Invocation::new("useradd", prefix);
            if *class == AccountClass::System {
                run = run.arg("--system");
            }
            run = run.arg("--no-create-home");
            if let Some(h) = home {
                run = run.arg("--home-dir").arg(h.as_str());
            }
            if let Some(s) = shell {
                run = run.arg("--shell").arg(s.as_str());
            }
            Plan::Run(run.name(name))
        }
        (UserCondition::Present { class, home, shell }, Some(u)) => {
            if AccountClass::of(u.uid) != *class {
                return Plan::Refuse;
            }
            let mut run = Invocation::new("usermod", prefix);
            let mut differs = false;
            if let Some(h) = home.as_ref().filter(|h| h.as_str() != u.home) {
                run = run.arg("--home").arg(h.as_str());
                differs = true;
            }
            if let Some(s) = shell.as_ref().filter(|s| s.as_str() != u.shell) {
                run = run.arg("--shell").arg(s.as_str());
                differs = true;
            }
            if differs {
                Plan::Run(run.name(name))
            } else {
                Plan::Nothing
            }
        }
    }
}

/// The receipt of a tool's run, from the account before and after.
pub fn receipt(ran: bool, before: &UserEvidence, after: &UserEvidence) -> Receipt {
    if ran {
        Receipt::Completed {
            changed: before != after,
        }
    } else {
        Receipt::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomos_core::resource::ResourcePath;

    fn args(plan: Plan) -> Vec<String> {
        let Plan::Run(run) = plan else {
            panic!("{plan:?}");
        };
        let mut out = vec![run.program.display().to_string()];
        out.extend(run.args.iter().map(|a| a.to_string_lossy().into_owned()));
        out
    }

    fn present(class: AccountClass, home: Option<&str>, shell: Option<&str>) -> UserCondition {
        UserCondition::Present {
            class,
            home: home.map(|h| ResourcePath::new(h).unwrap()),
            shell: shell.map(|s| ResourcePath::new(s).unwrap()),
        }
    }

    /// The command table of Users on Linux, written from the spec.
    #[test]
    fn each_requirement_runs_the_command_the_table_names() {
        let db = Accounts::parse(
            "root:x:0:0:root:/root:/bin/bash\napp:x:150:150::/var/lib/app:/usr/sbin/nologin\ntoor:x:0:0::/:/bin/sh\n",
            "",
        );
        let n = |s: &str| AccountName::new(s).unwrap();
        let root = Path::new("/srv/root");
        assert_eq!(
            args(plan(
                &db,
                &n("web"),
                &present(AccountClass::System, Some("/srv/web"), Some("/bin/sh")),
                None
            )),
            [
                "/usr/sbin/useradd",
                "--system",
                "--no-create-home",
                "--home-dir",
                "/srv/web",
                "--shell",
                "/bin/sh",
                "--",
                "web"
            ]
        );
        assert_eq!(
            args(plan(
                &db,
                &n("web"),
                &present(AccountClass::Regular, None, None),
                Some(root)
            )),
            [
                "/usr/sbin/useradd",
                "--prefix",
                "/srv/root",
                "--no-create-home",
                "--",
                "web"
            ]
        );
        assert_eq!(
            args(plan(
                &db,
                &n("app"),
                &present(AccountClass::System, Some("/var/lib/app"), Some("/bin/sh")),
                None
            )),
            ["/usr/sbin/usermod", "--shell", "/bin/sh", "--", "app"]
        );
        assert_eq!(
            plan(
                &db,
                &n("app"),
                &present(AccountClass::System, Some("/var/lib/app"), None),
                None
            ),
            Plan::Nothing
        );
        assert_eq!(
            args(plan(&db, &n("app"), &UserCondition::Absent, None)),
            ["/usr/sbin/userdel", "--", "app"]
        );
        assert_eq!(
            plan(&db, &n("web"), &UserCondition::Absent, None),
            Plan::Nothing
        );
        // The other class, and an ID another entry holds, are refused.
        assert_eq!(
            plan(
                &db,
                &n("app"),
                &present(AccountClass::Regular, None, None),
                None
            ),
            Plan::Refuse
        );
        assert_eq!(
            plan(&db, &n("root"), &UserCondition::Absent, None),
            Plan::Refuse
        );
    }
}
