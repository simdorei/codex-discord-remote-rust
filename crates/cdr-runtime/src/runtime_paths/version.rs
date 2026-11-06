use std::cmp::Ordering;

/// `SemVer` precedence, including numeric prerelease identifiers; build metadata is ignored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Version {
    numbers: [u64; 3],
    pre: Vec<String>,
}

impl Version {
    pub(super) fn parse(raw: &str) -> Option<Self> {
        let (raw, build) = raw
            .split_once('+')
            .map_or((raw, None), |(v, b)| (v, Some(b)));
        if build.is_some_and(|build| !identifiers(build, false)) {
            return None;
        }
        let (base, pre) = raw.split_once('-').map_or((raw, ""), |parts| parts);
        let parts: Vec<_> = base.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let mut numbers = [0; 3];
        for (index, value) in parts.iter().enumerate() {
            if !numeric(value) {
                return None;
            }
            numbers[index] = value.parse().ok()?;
        }
        if raw.contains('-') && !identifiers(pre, true) {
            return None;
        }
        Some(Self {
            numbers,
            pre: if pre.is_empty() {
                vec![]
            } else {
                pre.split('.').map(str::to_owned).collect()
            },
        })
    }
}

fn numeric(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|c| c.is_ascii_digit())
        && (value.len() == 1 || !value.starts_with('0'))
}

fn identifiers(value: &str, reject_zero: bool) -> bool {
    value.split('.').all(|part| {
        !part.is_empty()
            && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && (!reject_zero || !part.bytes().all(|c| c.is_ascii_digit()) || numeric(part))
    })
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let base = self.numbers.cmp(&other.numbers);
        if base != Ordering::Equal {
            return base;
        }
        if self.pre.is_empty() || other.pre.is_empty() {
            return self.pre.is_empty().cmp(&other.pre.is_empty());
        }
        for (left, right) in self.pre.iter().zip(&other.pre) {
            let numeric_left = numeric(left);
            let numeric_right = numeric(right);
            let order = match (numeric_left, numeric_right) {
                (true, true) => left.len().cmp(&right.len()).then(left.cmp(right)),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => left.cmp(right),
            };
            if order != Ordering::Equal {
                return order;
            }
        }
        self.pre.len().cmp(&other.pre.len())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::Version;
    #[test]
    fn semantic_precedence_and_invalid_inputs() {
        let order = [
            "0.166.0-alpha.9",
            "0.166.0-alpha.10",
            "0.166.0-beta",
            "0.166.0",
            "0.167.0",
        ];
        for pair in order.windows(2) {
            assert!(Version::parse(pair[0]) < Version::parse(pair[1]));
        }
        assert_eq!(
            Version::parse("0.166.0+build.1"),
            Version::parse("0.166.0+build.2")
        );
        for invalid in [
            "",
            "0.166",
            "0.0166.0",
            "0.166.0-",
            "0.166.0-alpha.01",
            "0.166.0+",
            "0.166.0\nextra",
        ] {
            assert!(Version::parse(invalid).is_none(), "{invalid}");
        }
    }
}
