//! Footprints and the composition check (ADR 0008 §1 and §2, with the
//! working definitions of its 2026-09-29 note and the Composition section
//! of `docs/formal/reconciliation.md`).
//!
//! A [`Footprint`] says what one controller writes, its guarantee, and what
//! it relies on no other controller writing, its reliance. Every written
//! property is relied on as well: a controller that converges alone on a
//! property does so because it is that property's only writer. [`compose`]
//! accepts footprints when no controller writes a property another relies
//! on, and otherwise names every [`Interference`].
//!
//! The check reads declarations only. It cannot see a controller that writes
//! outside its guarantee, and it compares properties as text, so two keys
//! that reach one operating-system object are two properties to it
//! (ADR 0008 §3).

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::resource::ResourcePath;

/// Why a string is not a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyError {
    /// The text is empty.
    Empty,
    /// The text contains whitespace.
    Whitespace,
    /// Nothing precedes the first `:`, or there is no `:`.
    NoKind,
    /// Nothing follows the first `:`.
    NoName,
}

impl fmt::Display for PropertyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PropertyError::Empty => "property is empty",
            PropertyError::Whitespace => "property contains whitespace",
            PropertyError::NoKind => "property has no kind before `:`",
            PropertyError::NoName => "property has no name after `:`",
        })
    }
}

/// One resource property, the unit of ownership: `<kind>:<name>`, for
/// example `file:/etc/example.conf#content` or `sysctl:vm.swappiness`.
/// Equality is textual.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Property(String);

impl Property {
    /// Validates `text` as `<kind>:<name>`, both non-empty, with no
    /// whitespace.
    pub fn new(text: &str) -> Result<Self, PropertyError> {
        if text.is_empty() {
            return Err(PropertyError::Empty);
        }
        if text.chars().any(char::is_whitespace) {
            return Err(PropertyError::Whitespace);
        }
        match text.split_once(':') {
            None | Some(("", _)) => Err(PropertyError::NoKind),
            Some((_, "")) => Err(PropertyError::NoName),
            Some(_) => Ok(Property(String::from(text))),
        }
    }

    /// The content of the file at `path`: `file:<path>#content`.
    pub fn file_content(path: &ResourcePath) -> Self {
        Property(format!("file:{path}#content"))
    }

    /// The property as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Property {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Property({})", self.0)
    }
}

impl fmt::Display for Property {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A controller's name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControllerId(String);

impl ControllerId {
    /// A controller by name, or `None` when `name` is empty.
    pub fn new(name: &str) -> Option<Self> {
        (!name.is_empty()).then(|| ControllerId(String::from(name)))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ControllerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ControllerId({})", self.0)
    }
}

impl fmt::Display for ControllerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What one controller writes, relies on, and reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footprint {
    controller: ControllerId,
    writes: BTreeSet<Property>,
    relies: BTreeSet<Property>,
    reads: BTreeSet<Property>,
}

impl Footprint {
    /// An empty footprint for `controller`: it writes, relies on, and reads
    /// nothing.
    pub fn new(controller: ControllerId) -> Self {
        Footprint {
            controller,
            writes: BTreeSet::new(),
            relies: BTreeSet::new(),
            reads: BTreeSet::new(),
        }
    }

    /// Adds `property` to the guarantee.
    #[must_use]
    pub fn writing(mut self, property: Property) -> Self {
        self.writes.insert(property);
        self
    }

    /// Adds `property` to the reliance without writing it.
    #[must_use]
    pub fn relying_on(mut self, property: Property) -> Self {
        self.relies.insert(property);
        self
    }

    /// Adds `property` to the reads: observed, and tolerated to change
    /// under another controller's guarantee.
    #[must_use]
    pub fn reading(mut self, property: Property) -> Self {
        self.reads.insert(property);
        self
    }

    /// The controller.
    pub fn controller(&self) -> &ControllerId {
        &self.controller
    }

    /// The guarantee: the controller writes no property outside it.
    pub fn writes(&self) -> &BTreeSet<Property> {
        &self.writes
    }

    /// The reliance: what the controller relies on no other controller
    /// writing. It always includes the guarantee.
    pub fn reliance(&self) -> BTreeSet<Property> {
        self.writes.union(&self.relies).cloned().collect()
    }

    /// The reads.
    pub fn reads(&self) -> &BTreeSet<Property> {
        &self.reads
    }
}

/// Why footprints do not compose.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Interference {
    /// Two footprints name one controller.
    DuplicateController(ControllerId),
    /// Two controllers write `property`, so neither's reliance holds. The
    /// writers are in name order.
    SharedWrite {
        /// The property.
        property: Property,
        /// The two writers, in name order.
        writers: (ControllerId, ControllerId),
    },
    /// `writer` writes `property`, which `relying` relies on and does not
    /// write.
    RelianceViolated {
        /// The property.
        property: Property,
        /// The controller that relies on it.
        relying: ControllerId,
        /// The controller that writes it.
        writer: ControllerId,
    },
}

/// The composition check: `Ok` when the controllers are distinct and no
/// controller writes a property another relies on; otherwise every
/// interference, sorted, so the result does not depend on the order of
/// `footprints`.
pub fn compose(footprints: &[Footprint]) -> Result<(), Vec<Interference>> {
    let mut found = BTreeSet::new();
    for (i, a) in footprints.iter().enumerate() {
        for b in footprints.iter().skip(i + 1) {
            if a.controller == b.controller {
                found.insert(Interference::DuplicateController(a.controller.clone()));
                continue;
            }
            for (relying, writer) in [(a, b), (b, a)] {
                for property in relying.reliance().intersection(&writer.writes) {
                    found.insert(if relying.writes.contains(property) {
                        let pair = if a.controller < b.controller {
                            (a.controller.clone(), b.controller.clone())
                        } else {
                            (b.controller.clone(), a.controller.clone())
                        };
                        Interference::SharedWrite {
                            property: property.clone(),
                            writers: pair,
                        }
                    } else {
                        Interference::RelianceViolated {
                            property: property.clone(),
                            relying: relying.controller.clone(),
                            writer: writer.controller.clone(),
                        }
                    });
                }
            }
        }
    }
    if found.is_empty() {
        Ok(())
    } else {
        Err(found.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn id(name: &str) -> ControllerId {
        ControllerId::new(name).unwrap()
    }

    fn prop(text: &str) -> Property {
        Property::new(text).unwrap()
    }

    #[test]
    fn properties_are_kind_and_name() {
        for text in ["file:/etc/a#content", "sysctl:vm.swappiness", "k:n:m"] {
            assert_eq!(prop(text).as_str(), text);
        }
        let cases = [
            ("", PropertyError::Empty),
            ("file:/etc/a b", PropertyError::Whitespace),
            ("nokind", PropertyError::NoKind),
            (":name", PropertyError::NoKind),
            ("kind:", PropertyError::NoName),
        ];
        for (text, error) in cases {
            assert_eq!(Property::new(text).unwrap_err(), error, "{text:?}");
            assert!(alloc::format!("{error}").contains("property"));
        }
        let path = ResourcePath::new("/etc/a").unwrap();
        assert_eq!(Property::file_content(&path), prop("file:/etc/a#content"));
        assert_eq!(
            alloc::format!("{:?}", prop("k:n")),
            "Property(k:n)",
            "debug form"
        );
        assert_eq!(alloc::format!("{}", prop("k:n")), "k:n");
        assert!(ControllerId::new("").is_none());
        assert_eq!(
            alloc::format!("{:?} {}", id("a"), id("a")),
            "ControllerId(a) a"
        );
    }

    #[test]
    fn a_write_is_relied_on() {
        let footprint = Footprint::new(id("a"))
            .writing(prop("k:w"))
            .relying_on(prop("k:r"))
            .reading(prop("k:x"));
        assert_eq!(
            footprint.reliance(),
            [prop("k:r"), prop("k:w")].into_iter().collect()
        );
        assert_eq!(footprint.writes(), &[prop("k:w")].into_iter().collect());
        assert_eq!(footprint.reads(), &[prop("k:x")].into_iter().collect());
        assert_eq!(footprint.controller(), &id("a"));
    }

    /// What one controller declares about the one property of the table.
    #[derive(Clone, Copy, Debug)]
    enum Role {
        None,
        Read,
        Rely,
        Write,
    }

    fn with(controller: &str, role: Role) -> Footprint {
        let f = Footprint::new(id(controller));
        match role {
            Role::None => f,
            Role::Read => f.reading(prop("k:p")),
            Role::Rely => f.relying_on(prop("k:p")),
            Role::Write => f.writing(prop("k:p")),
        }
    }

    /// The exhaustive truth table over two controllers and one property,
    /// written from the working definitions of ADR 0008's 2026-09-29 note,
    /// not from the code: rejected exactly when one controller writes what
    /// the other relies on, and a write is relied on.
    #[test]
    fn two_controllers_on_one_property_truth_table() {
        use Role::*;
        #[derive(Debug, PartialEq)]
        enum Verdict {
            Accepted,
            Shared,
            /// The first-named controller relies; the second writes.
            Violated(&'static str, &'static str),
        }
        use Verdict::*;
        let table = [
            (None, None, Accepted),
            (None, Read, Accepted),
            (None, Rely, Accepted),
            (None, Write, Accepted),
            (Read, None, Accepted),
            (Read, Read, Accepted),
            (Read, Rely, Accepted),
            (Read, Write, Accepted),
            (Rely, None, Accepted),
            (Rely, Read, Accepted),
            (Rely, Rely, Accepted),
            (Rely, Write, Violated("a", "b")),
            (Write, None, Accepted),
            (Write, Read, Accepted),
            (Write, Rely, Violated("b", "a")),
            (Write, Write, Shared),
        ];
        for (ra, rb, expected) in table {
            let got = compose(&[with("a", ra), with("b", rb)]);
            let want = match expected {
                Accepted => Ok(()),
                Shared => Err(vec![Interference::SharedWrite {
                    property: prop("k:p"),
                    writers: (id("a"), id("b")),
                }]),
                Violated(relying, writer) => Err(vec![Interference::RelianceViolated {
                    property: prop("k:p"),
                    relying: id(relying),
                    writer: id(writer),
                }]),
            };
            assert_eq!(got, want, "a {ra:?}, b {rb:?}");
        }
    }

    /// Metamorphic relations: the verdict does not depend on the order of
    /// the footprints, and adding a footprint disjoint from the rest does not
    /// change it.
    #[test]
    fn order_and_disjoint_additions_do_not_change_the_verdict() {
        let a = Footprint::new(id("a"))
            .writing(prop("k:1"))
            .writing(prop("k:2"))
            .relying_on(prop("k:3"));
        let b = Footprint::new(id("b"))
            .writing(prop("k:2"))
            .writing(prop("k:3"));
        let c = Footprint::new(id("c"))
            .writing(prop("k:1"))
            .reading(prop("k:3"));
        let d = Footprint::new(id("d")).writing(prop("k:9"));
        let orders = [
            vec![a.clone(), b.clone(), c.clone()],
            vec![c.clone(), b.clone(), a.clone()],
            vec![b.clone(), a.clone(), c.clone()],
            vec![b.clone(), d.clone(), c.clone(), a.clone()],
            vec![d.clone(), a.clone(), c.clone(), b.clone()],
        ];
        let expected = Err(vec![
            Interference::SharedWrite {
                property: prop("k:1"),
                writers: (id("a"), id("c")),
            },
            Interference::SharedWrite {
                property: prop("k:2"),
                writers: (id("a"), id("b")),
            },
            Interference::RelianceViolated {
                property: prop("k:3"),
                relying: id("a"),
                writer: id("b"),
            },
        ]);
        for footprints in orders {
            assert_eq!(compose(&footprints), expected);
        }
        assert_eq!(compose(&[a.clone(), d.clone()]), Ok(()));
        assert_eq!(compose(&[]), Ok(()));
        assert_eq!(compose(&[a]), Ok(()), "a controller alone composes");
    }

    #[test]
    fn a_controller_named_twice_is_rejected() {
        let a = Footprint::new(id("a")).writing(prop("k:1"));
        let again = Footprint::new(id("a")).writing(prop("k:2"));
        assert_eq!(
            compose(&[a, again]),
            Err(vec![Interference::DuplicateController(id("a"))])
        );
    }
}
