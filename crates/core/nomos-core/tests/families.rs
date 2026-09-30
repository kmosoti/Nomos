//! The truth tables of every resource family, transcribed from
//! `docs/formal/resource-families.md` and checked exhaustively: every
//! requirement shape against every evidence shape. Their oracle is the
//! written table, not the implementation (ADR 0015 §5). The file family's
//! content table is `tests/laws.rs`'s; this file adds its metadata rows.

use nomos_core::assessment::{
    AccountField, Assessment, MetadataField, Reason, Variance, assess, assess_collection,
    assess_directory, assess_evidence, assess_file, assess_package, assess_sysctl, assess_unit,
    assess_user,
};
use nomos_core::condition::{
    AccountClass, Activity, Condition, Content, DirectoryCondition, Enablement, FileCondition,
    Metadata, PackageCondition, PackageVersion, Requirement, SysctlCondition, SysctlValue,
    UnitCondition, UserCondition,
};
use nomos_core::observation::{
    Account, ActiveState, Collection, CollectionFailure, CollectorId, DirectoryEvidence, Evidence,
    FileEvidence, Instant, Observation, ObservedMetadata, PackageEvidence, Provenance,
    SysctlEvidence, UnitEvidence, UnitFileState, UserEvidence, Window,
};
use nomos_core::resource::{
    AccountName, Digest, Family, Mode, PackageName, ResourceKey, ResourcePath, SysctlKey, UnitName,
};

fn name(text: &str) -> AccountName {
    AccountName::new(text).unwrap()
}

fn path(text: &str) -> ResourcePath {
    ResourcePath::new(text).unwrap()
}

fn mode(bits: u16) -> Mode {
    Mode::new(bits).unwrap()
}

fn observed(owner: Account, group: Account, bits: u16) -> ObservedMetadata {
    ObservedMetadata {
        owner,
        group,
        mode: mode(bits),
    }
}

fn provenance() -> Provenance {
    Provenance::new(
        CollectorId::new("families").unwrap(),
        Window::new(Instant(0), Instant(1)).unwrap(),
    )
}

// ---------------------------------------------------------------------------
// Metadata, shared by files and directories

/// Every metadata requirement against every observed metadata: a stated
/// field must equal the observed one, an owner or group given by number
/// never equals a name, and the first differing field is reported in the
/// order owner, group, mode.
#[test]
fn metadata_rows_of_the_file_and_directory_tables() {
    let www = || Account::Named(name("www-data"));
    let root = || Account::Named(name("root"));
    let requirements = [
        (Metadata::any(), None),
        (
            Metadata {
                owner: Some(name("www-data")),
                ..Metadata::any()
            },
            Some(MetadataField::Owner),
        ),
        (
            Metadata {
                group: Some(name("www-data")),
                ..Metadata::any()
            },
            Some(MetadataField::Group),
        ),
        (
            Metadata {
                mode: Some(mode(0o600)),
                ..Metadata::any()
            },
            Some(MetadataField::Mode),
        ),
        (
            Metadata {
                owner: Some(name("www-data")),
                group: Some(name("www-data")),
                mode: Some(mode(0o600)),
            },
            Some(MetadataField::Owner),
        ),
    ];
    // Observed: root:root 0644 differs from every stated field.
    let all_differ = observed(root(), root(), 0o644);
    // Observed: www-data:www-data 0600 matches every stated field.
    let all_match = observed(www(), www(), 0o600);
    // Observed by number: never equal to a name.
    let unnamed = observed(Account::Id(33), Account::Id(33), 0o600);
    let content = Digest::from_bytes([1; 32]);
    for (metadata, first_differing) in requirements {
        let file = FileCondition::Present {
            content: Content::Exactly(content),
            metadata: metadata.clone(),
        };
        let dir = DirectoryCondition::Present {
            metadata: metadata.clone(),
        };
        let file_ev = |m: &ObservedMetadata| FileEvidence::Present {
            digest: content,
            size: 1,
            metadata: m.clone(),
        };
        let dir_ev = |m: &ObservedMetadata| DirectoryEvidence::Present {
            metadata: m.clone(),
        };
        let differing = first_differing.map_or(Assessment::Satisfied, |f| {
            Assessment::Variance(Variance::MetadataDiffers(f))
        });
        assert_eq!(
            assess_file(&file, &file_ev(&all_differ)),
            differing,
            "{metadata:?}"
        );
        assert_eq!(
            assess_directory(&dir, &dir_ev(&all_differ)),
            differing,
            "{metadata:?}"
        );
        assert_eq!(
            assess_file(&file, &file_ev(&all_match)),
            Assessment::Satisfied
        );
        assert_eq!(
            assess_directory(&dir, &dir_ev(&all_match)),
            Assessment::Satisfied
        );
        let by_number = match first_differing {
            Some(MetadataField::Mode) | None => Assessment::Satisfied,
            Some(f) => Assessment::Variance(Variance::MetadataDiffers(f)),
        };
        assert_eq!(
            assess_file(&file, &file_ev(&unnamed)),
            by_number,
            "{metadata:?}"
        );
    }
}

/// Content is judged before metadata: wrong bytes report `content-differs`
/// whatever the metadata.
#[test]
fn content_is_judged_before_metadata() {
    let requirement = FileCondition::Present {
        content: Content::Exactly(Digest::from_bytes([1; 32])),
        metadata: Metadata {
            mode: Some(mode(0o600)),
            ..Metadata::any()
        },
    };
    let evidence = FileEvidence::Present {
        digest: Digest::from_bytes([2; 32]),
        size: 1,
        metadata: observed(Account::Id(0), Account::Id(0), 0o644),
    };
    assert_eq!(
        assess_file(&requirement, &evidence),
        Assessment::Variance(Variance::ContentDiffers {
            expected: Digest::from_bytes([1; 32]),
            observed: Digest::from_bytes([2; 32]),
        })
    );
}

#[test]
fn the_directory_table_presence_rows() {
    let present = DirectoryCondition::Present {
        metadata: Metadata::any(),
    };
    let seen = DirectoryEvidence::Present {
        metadata: ObservedMetadata::root_default(),
    };
    let table = [
        (
            &DirectoryCondition::Absent,
            &DirectoryEvidence::Absent,
            Assessment::Satisfied,
        ),
        (
            &DirectoryCondition::Absent,
            &seen,
            Assessment::Variance(Variance::Unexpected),
        ),
        (
            &present,
            &DirectoryEvidence::Absent,
            Assessment::Variance(Variance::Missing),
        ),
        (&present, &seen, Assessment::Satisfied),
    ];
    for (r, e, expected) in table {
        assert_eq!(assess_directory(r, e), expected, "{r:?} vs {e:?}");
    }
}

// ---------------------------------------------------------------------------
// Units

const ACTIVE_STATES: [ActiveState; 6] = [
    ActiveState::Active,
    ActiveState::Reloading,
    ActiveState::Inactive,
    ActiveState::Failed,
    ActiveState::Activating,
    ActiveState::Deactivating,
];

const FILE_STATES: [UnitFileState; 5] = [
    UnitFileState::Enabled,
    UnitFileState::Disabled,
    UnitFileState::Static,
    UnitFileState::Masked,
    UnitFileState::Other,
];

/// The unit table, read directly: which observed states satisfy each axis.
fn activity_holds(required: Activity, observed: ActiveState) -> bool {
    match required {
        Activity::Any => true,
        Activity::Active => [ActiveState::Active, ActiveState::Reloading].contains(&observed),
        Activity::Inactive => [ActiveState::Inactive, ActiveState::Failed].contains(&observed),
    }
}

fn enablement_holds(required: Enablement, observed: UnitFileState) -> bool {
    match required {
        Enablement::Any => true,
        Enablement::Enabled => observed == UnitFileState::Enabled,
        Enablement::Disabled => observed == UnitFileState::Disabled,
    }
}

/// Every unit requirement against every unit evidence: 9 × 30 cases.
#[test]
fn the_unit_table_is_exhaustive() {
    let mut cases = 0;
    for activity in [Activity::Active, Activity::Inactive, Activity::Any] {
        for enablement in [Enablement::Enabled, Enablement::Disabled, Enablement::Any] {
            for active in ACTIVE_STATES {
                for file_state in FILE_STATES {
                    let r = UnitCondition {
                        activity,
                        enablement,
                    };
                    let e = UnitEvidence { active, file_state };
                    let a_ok = activity_holds(activity, active);
                    let e_ok = enablement_holds(enablement, file_state);
                    let expected = if a_ok && e_ok {
                        Assessment::Satisfied
                    } else {
                        Assessment::Variance(Variance::UnitDiffers {
                            activity: !a_ok,
                            enablement: !e_ok,
                        })
                    };
                    assert_eq!(assess_unit(&r, &e), expected, "{r:?} vs {e:?}");
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 9 * 30);
}

// ---------------------------------------------------------------------------
// Sysctl

#[test]
fn a_sysctl_value_is_compared_after_normalization() {
    let r = |v: &str| SysctlCondition {
        value: SysctlValue::normalized(v),
    };
    let e = |v: &str| SysctlEvidence {
        value: SysctlValue::normalized(v),
    };
    assert_eq!(assess_sysctl(&r("1"), &e("1\n")), Assessment::Satisfied);
    assert_eq!(
        assess_sysctl(&r("4096 87380 6291456"), &e("4096\t87380   6291456\n")),
        Assessment::Satisfied
    );
    assert_eq!(
        assess_sysctl(&r("1"), &e("0")),
        Assessment::Variance(Variance::ValueDiffers)
    );
    assert_eq!(
        assess_sysctl(&r("1 2"), &e("1")),
        Assessment::Variance(Variance::ValueDiffers)
    );
    assert!(SysctlValue::is_normal("4096 87380"));
    assert!(!SysctlValue::is_normal(" 1"));
    assert!(!SysctlValue::is_normal("1  2"));
}

// ---------------------------------------------------------------------------
// Users

#[test]
fn the_user_table() {
    let present = |class, home: Option<&str>, shell: Option<&str>| UserCondition::Present {
        class,
        home: home.map(path),
        shell: shell.map(path),
    };
    let seen = |uid, home: &str, shell: &str| UserEvidence::Present {
        uid,
        gid: uid,
        home: home.into(),
        shell: shell.into(),
    };
    let system = seen(998, "/var/lib/app", "/usr/sbin/nologin");
    let regular = seen(1000, "/home/app", "/bin/bash");
    let table = [
        (
            UserCondition::Absent,
            UserEvidence::Absent,
            Assessment::Satisfied,
        ),
        (
            UserCondition::Absent,
            system.clone(),
            Assessment::Variance(Variance::Unexpected),
        ),
        (
            present(AccountClass::System, None, None),
            UserEvidence::Absent,
            Assessment::Variance(Variance::Missing),
        ),
        (
            present(AccountClass::System, None, None),
            system.clone(),
            Assessment::Satisfied,
        ),
        (
            present(AccountClass::System, None, None),
            regular.clone(),
            Assessment::Variance(Variance::AccountDiffers(AccountField::Class)),
        ),
        (
            present(AccountClass::Regular, None, None),
            regular.clone(),
            Assessment::Satisfied,
        ),
        (
            present(AccountClass::System, Some("/srv/app"), Some("/bin/sh")),
            system.clone(),
            Assessment::Variance(Variance::AccountDiffers(AccountField::Home)),
        ),
        (
            present(AccountClass::System, Some("/var/lib/app"), Some("/bin/sh")),
            system.clone(),
            Assessment::Variance(Variance::AccountDiffers(AccountField::Shell)),
        ),
        (
            present(
                AccountClass::System,
                Some("/var/lib/app"),
                Some("/usr/sbin/nologin"),
            ),
            system,
            Assessment::Satisfied,
        ),
    ];
    for (r, e, expected) in table {
        assert_eq!(assess_user(&r, &e), expected, "{r:?} vs {e:?}");
    }
    // The class boundary is Debian's SYS_UID_MAX.
    assert_eq!(AccountClass::of(0), AccountClass::System);
    assert_eq!(AccountClass::of(999), AccountClass::System);
    assert_eq!(AccountClass::of(1000), AccountClass::Regular);
}

// ---------------------------------------------------------------------------
// Packages

#[test]
fn the_package_table_is_exhaustive() {
    let v = |t: &str| PackageVersion::new(t).unwrap();
    let requirements = [
        PackageCondition::Absent,
        PackageCondition::Installed { version: None },
        PackageCondition::Installed {
            version: Some(v("1.2-1")),
        },
    ];
    let evidences = [
        PackageEvidence::NotInstalled,
        PackageEvidence::Installed {
            version: v("1.2-1"),
        },
        PackageEvidence::Installed {
            version: v("1.3-1"),
        },
        PackageEvidence::Broken,
    ];
    let expected = [
        // absent
        [
            Assessment::Satisfied,
            Assessment::Variance(Variance::Unexpected),
            Assessment::Variance(Variance::Unexpected),
            Assessment::Variance(Variance::Broken),
        ],
        // installed, any version
        [
            Assessment::Variance(Variance::Missing),
            Assessment::Satisfied,
            Assessment::Satisfied,
            Assessment::Variance(Variance::Broken),
        ],
        // installed at 1.2-1
        [
            Assessment::Variance(Variance::Missing),
            Assessment::Satisfied,
            Assessment::Variance(Variance::VersionDiffers),
            Assessment::Variance(Variance::Broken),
        ],
    ];
    for (r, row) in requirements.iter().zip(expected) {
        for (e, want) in evidences.iter().zip(row) {
            assert_eq!(assess_package(r, e), want, "{r:?} vs {e:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Across families

/// N13 in every family: a failed collection is Indeterminate with its
/// reason, never a Variance, including `unavailable`.
#[test]
fn a_failed_collection_is_indeterminate_in_every_family() {
    let requirements = [
        Requirement::Directory(DirectoryCondition::Absent),
        Requirement::File(FileCondition::Absent),
        Requirement::Package(PackageCondition::Absent),
        Requirement::Service(FileCondition::Absent),
        Requirement::Sysctl(SysctlCondition {
            value: SysctlValue::normalized("1"),
        }),
        Requirement::Unit(UnitCondition {
            activity: Activity::Active,
            enablement: Enablement::Any,
        }),
        Requirement::User(UserCondition::Absent),
    ];
    let failures = [
        CollectionFailure::PermissionDenied,
        CollectionFailure::TimedOut,
        CollectionFailure::Unsupported,
        CollectionFailure::Io,
        CollectionFailure::Unavailable,
    ];
    for r in &requirements {
        for f in failures {
            assert_eq!(
                assess_collection(r, &Collection::Failed(f)),
                Assessment::Indeterminate(Reason::CollectionFailed(f)),
                "{r:?} {f:?}"
            );
        }
    }
    let families: Vec<Family> = requirements.iter().map(Requirement::family).collect();
    assert_eq!(
        families,
        Family::ALL,
        "one requirement per family, in order"
    );
}

/// Evidence of another family is never a verdict: `assess_evidence` says
/// so, and `assess_collection` calls it Indeterminate, not a Variance.
#[test]
fn evidence_of_another_family_is_never_a_verdict() {
    let file = Requirement::File(FileCondition::Absent);
    let unit_evidence = Evidence::Unit(UnitEvidence {
        active: ActiveState::Active,
        file_state: UnitFileState::Enabled,
    });
    assert_eq!(assess_evidence(&file, &unit_evidence), None);
    assert_eq!(
        assess_collection(&file, &Collection::Collected(unit_evidence)),
        Assessment::Indeterminate(Reason::WrongFamily)
    );
}

/// A Condition's requirement is of its key's family, and an Observation's
/// evidence of its key's family: the fallible constructors refuse anything
/// else, and `assess` only pairs a Condition with Observations of its key.
#[test]
fn keys_requirements_and_evidence_agree_on_the_family() {
    let unit = UnitName::new("nginx.service").unwrap();
    let key = ResourceKey::Unit(unit.clone());
    assert!(Condition::new(key.clone(), Requirement::File(FileCondition::Absent)).is_none());
    assert!(
        Observation::new(
            key.clone(),
            Collection::Collected(Evidence::File(FileEvidence::Absent)),
            provenance()
        )
        .is_none()
    );
    assert!(
        Observation::new(
            key.clone(),
            Collection::Failed(CollectionFailure::Unavailable),
            provenance()
        )
        .is_some()
    );
    // A file and a unit observation of the "same name" are different keys.
    let condition = Condition::unit(
        unit.clone(),
        UnitCondition {
            activity: Activity::Active,
            enablement: Enablement::Any,
        },
    );
    let elsewhere = Observation::file(
        path("/etc/nginx.service"),
        Collection::Collected(FileEvidence::Absent),
        provenance(),
    );
    assert_eq!(
        assess(&condition, &[elsewhere]),
        Assessment::Indeterminate(Reason::NoObservation)
    );
    let seen = Observation::unit(
        unit,
        Collection::Collected(UnitEvidence {
            active: ActiveState::Inactive,
            file_state: UnitFileState::Enabled,
        }),
        provenance(),
    );
    assert_eq!(
        assess(&condition, &[seen]),
        Assessment::Variance(Variance::UnitDiffers {
            activity: true,
            enablement: false
        })
    );
}

// ---------------------------------------------------------------------------
// Names

#[test]
fn names_are_valid_for_their_family_and_nothing_else_is() {
    for ok in [
        "nginx.service",
        "getty@tty1.service",
        "a.timer",
        "x-y_z:1.mount",
        "dev-sda1.path",
    ] {
        assert!(UnitName::new(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        ".service",
        "nginx",
        "nginx.conf",
        "a/b.service",
        "a b.service",
    ] {
        assert!(UnitName::new(bad).is_err(), "{bad}");
    }
    for ok in [
        "net.ipv4.ip_forward",
        "vm.swappiness",
        "kernel.core_pattern",
        "net.ipv4.conf.all.rp_filter",
    ] {
        assert!(SysctlKey::new(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "vm",
        "vm.",
        ".vm.x",
        "net/ipv4/ip_forward",
        "Net.x",
        "vm..x",
    ] {
        assert!(SysctlKey::new(bad).is_err(), "{bad}");
    }
    for ok in ["root", "www-data", "_apt", "systemd-network", "host$", "a1"] {
        assert!(AccountName::new(ok).is_ok(), "{ok}");
    }
    for bad in ["", "Root", "1abc", "-x", "a b", "$", &"a".repeat(33)] {
        assert!(AccountName::new(bad).is_err(), "{bad}");
    }
    for ok in ["nginx", "libc6", "g++", "python3.11", "0ad"] {
        assert!(PackageName::new(ok).is_ok(), "{ok}");
    }
    for bad in ["", "a", "Nginx", "-x", "a_b", "a b"] {
        assert!(PackageName::new(bad).is_err(), "{bad}");
    }
    for ok in ["1.2-1", "2:1.18.0-6+deb12u2", "0.1~rc1"] {
        assert!(PackageVersion::new(ok).is_some(), "{ok}");
    }
    for bad in ["", "v1.2", "1.2 3", "1/2"] {
        assert!(PackageVersion::new(bad).is_none(), "{bad}");
    }
    assert_eq!(Mode::from_octal("0644"), Some(mode(0o644)));
    assert_eq!(Mode::from_octal("7777"), Some(mode(0o7777)));
    for bad in ["644", "0648", "10000", "", "0o64"] {
        assert_eq!(Mode::from_octal(bad), None, "{bad}");
    }
    assert_eq!(format!("{}", mode(0o644)), "0644");
}

/// Keys order by name, then family: paths first, in path order, whatever
/// their family, then the other names. Printing and parsing agree.
#[test]
fn keys_parse_print_and_order_consistently() {
    let texts = [
        "file:/etc/hosts",
        "service:/run/svc",
        "directory:/srv",
        "file:/zz",
        "package:nginx",
        "user:nginx",
        "unit:nginx.service",
        "sysctl:vm.swappiness",
        "user:www-data",
    ];
    let keys: Vec<ResourceKey> = texts
        .iter()
        .map(|t| ResourceKey::parse(t).unwrap())
        .collect();
    for (key, text) in keys.iter().zip(texts) {
        assert_eq!(key.to_string(), text);
    }
    let mut sorted = keys.clone();
    sorted.reverse();
    sorted.sort();
    assert_eq!(sorted, keys);
    // One name in two families: the family decides, in the order of their
    // names as text.
    let package = ResourceKey::parse("package:nginx").unwrap();
    let user = ResourceKey::parse("user:nginx").unwrap();
    assert!(package < user);
    let mut names: Vec<&str> = Family::ALL.iter().map(Family::as_str).collect();
    let before = names.clone();
    names.sort_unstable();
    assert_eq!(names, before);
    assert!(ResourceKey::parse("nokind").is_err());
    assert!(ResourceKey::parse("socket:/x").is_err());
    assert!(ResourceKey::parse("file:relative").is_err());
    assert!(ResourceKey::parse("unit:nginx").is_err());
}
