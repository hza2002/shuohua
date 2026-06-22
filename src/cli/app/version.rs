use semver::Version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDecision {
    Current,
    Update { from: Version, to: Version },
    RefuseMajor { from: Version, to: Version },
}

pub fn parse_release_tag(tag: &str) -> anyhow::Result<Version> {
    let raw = tag
        .strip_prefix('v')
        .ok_or_else(|| anyhow::anyhow!("release tag must use vX.Y.Z format: {tag}"))?;
    let version = Version::parse(raw)?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        anyhow::bail!("release tag must use stable vX.Y.Z format: {tag}");
    }
    Ok(version)
}

pub fn decide(current: &Version, latest: &Version, allow_major: bool) -> UpdateDecision {
    if latest <= current {
        return UpdateDecision::Current;
    }
    if latest.major != current.major && !allow_major {
        return UpdateDecision::RefuseMajor {
            from: current.clone(),
            to: latest.clone(),
        };
    }
    UpdateDecision::Update {
        from: current.clone(),
        to: latest.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v_prefixed_release_tags() {
        assert_eq!(
            parse_release_tag("v0.2.3").unwrap(),
            Version::parse("0.2.3").unwrap()
        );
    }

    #[test]
    fn rejects_unprefixed_release_tags() {
        let err = parse_release_tag("0.2.3").unwrap_err();
        assert!(err.to_string().contains("vX.Y.Z"), "{err:#}");
    }

    #[test]
    fn rejects_prerelease_and_build_release_tags() {
        for tag in ["v1.2.3-rc.1", "v1.2.3+meta"] {
            let err = parse_release_tag(tag).unwrap_err();
            assert!(err.to_string().contains("stable vX.Y.Z"), "{err:#}");
        }
    }

    #[test]
    fn allows_zero_minor_updates() {
        let current = Version::parse("0.1.2").unwrap();
        let latest = Version::parse("0.2.0").unwrap();

        assert_eq!(
            decide(&current, &latest, false),
            UpdateDecision::Update {
                from: current,
                to: latest
            }
        );
    }

    #[test]
    fn refuses_major_updates_by_default() {
        let current = Version::parse("0.9.9").unwrap();
        let latest = Version::parse("1.0.0").unwrap();

        assert_eq!(
            decide(&current, &latest, false),
            UpdateDecision::RefuseMajor {
                from: current,
                to: latest
            }
        );
    }

    #[test]
    fn allows_major_updates_when_requested() {
        let current = Version::parse("1.9.9").unwrap();
        let latest = Version::parse("2.0.0").unwrap();

        assert_eq!(
            decide(&current, &latest, true),
            UpdateDecision::Update {
                from: current,
                to: latest
            }
        );
    }
}
