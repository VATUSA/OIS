//! Who may write a release, and on what condition (VATUSA/OIS#585).
//!
//! OIS is the system of record, and an external tool's writes are proposals. People keep today's
//! behaviour: they may create, replace or clear any release, including a machine's. A machine (a
//! service account or a user's API key) may create a release for an unheld flight and replace or
//! clear **only its own** — never one a person holds, and never another machine's.
//!
//! A machine's every write is also conditional, so a retried request cannot issue a committed time
//! twice and a stale one cannot replace a newer one: `If-None-Match: *` means "only if there is no
//! release", `If-Match: N` means "only if it is still version N". People may send them too, but
//! needn't, which keeps the web UI unchanged.
//!
//! The written contract is `docs/features/flow.md` § "External release writers".

use axum::http::{HeaderMap, HeaderValue, header};

use crate::{
    auth::principal::Principal,
    errors::ApiError,
    repos::flow::{Expect, ReleaseHolder},
};

fn is_machine(principal: &Principal) -> bool {
    matches!(
        principal,
        Principal::ServiceAccount(_) | Principal::ApiKey(_)
    )
}

/// The writer's precondition, from `If-Match` / `If-None-Match`. `None` when it sent neither.
///
/// `If-Match` takes one version, quoted or bare (`"3"` or `3`). A weak tag, a list, or anything
/// else is a 400 rather than a guess: a precondition that silently matched nothing would let a
/// retry through.
pub fn parse_precondition(headers: &HeaderMap) -> Result<Option<Expect>, ApiError> {
    if let Some(value) = headers.get(header::IF_NONE_MATCH) {
        return match value.to_str().map(str::trim) {
            Ok("*") => Ok(Some(Expect::Absent)),
            _ => Err(ApiError::BadRequest),
        };
    }
    let Some(value) = headers.get(header::IF_MATCH) else {
        return Ok(None);
    };
    let raw = value.to_str().map_err(|_| ApiError::BadRequest)?.trim();
    let bare = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(raw);
    bare.parse::<i64>()
        .map(|v| Some(Expect::Version(v)))
        .map_err(|_| ApiError::BadRequest)
}

/// [`parse_precondition`], with a machine required to send one (428 otherwise).
pub fn precondition(headers: &HeaderMap, writer: &Principal) -> Result<Option<Expect>, ApiError> {
    let expect = parse_precondition(headers)?;
    if expect.is_none() && is_machine(writer) {
        return Err(ApiError::PreconditionRequired);
    }
    Ok(expect)
}

/// Whether `writer` (whose own actor id is `writer_actor`) may change a release `holder` holds.
pub fn authorize(
    writer: &Principal,
    writer_actor: Option<&str>,
    holder: Option<&ReleaseHolder>,
) -> Result<(), ApiError> {
    if !is_machine(writer) {
        return Ok(());
    }
    match holder.map(|h| h.machine.as_ref()) {
        None => Ok(()),
        Some(None) => Err(ApiError::ConflictReason("held_by_person")),
        Some(Some((holder_actor, _))) if Some(holder_actor.as_str()) == writer_actor => Ok(()),
        Some(Some(_)) => Err(ApiError::ConflictReason("held_by_other_machine")),
    }
}

/// The `ETag` for a release at `version`.
pub fn etag(version: i64) -> [(header::HeaderName, HeaderValue); 1] {
    let value =
        HeaderValue::from_str(&format!("\"{version}\"")).expect("digits are a valid header");
    [(header::ETAG, value)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::{CurrentApiKey, CurrentServiceAccount};
    use crate::scope_test_support::principal_for;

    fn machine() -> Principal {
        Principal::ServiceAccount(CurrentServiceAccount {
            id: "sa".into(),
            key: "vtbfm".into(),
            name: "vTBFM".into(),
        })
    }

    fn key() -> Principal {
        Principal::ApiKey(CurrentApiKey {
            id: "k".into(),
            owner_user_id: "owner".into(),
            prefix: "ois_pat_x".into(),
            name: "k".into(),
        })
    }

    fn held_by_person() -> ReleaseHolder {
        ReleaseHolder {
            version: 1,
            machine: None,
        }
    }

    fn held_by(actor: &str) -> ReleaseHolder {
        ReleaseHolder {
            version: 1,
            machine: Some((actor.into(), "vTBFM".into())),
        }
    }

    /// The whole authority table.
    #[test]
    fn people_may_write_anything() {
        let person = principal_for("u1");
        for holder in [None, Some(held_by_person()), Some(held_by("sa-actor"))] {
            assert!(authorize(&person, Some("u-actor"), holder.as_ref()).is_ok());
        }
    }

    #[test]
    fn a_machine_may_create_and_may_replace_its_own() {
        for writer in [machine(), key()] {
            assert!(authorize(&writer, Some("me"), None).is_ok());
            assert!(authorize(&writer, Some("me"), Some(&held_by("me"))).is_ok());
        }
    }

    #[test]
    fn a_machine_may_not_touch_a_persons_release() {
        for writer in [machine(), key()] {
            assert!(matches!(
                authorize(&writer, Some("me"), Some(&held_by_person())),
                Err(ApiError::ConflictReason("held_by_person"))
            ));
        }
    }

    #[test]
    fn a_machine_may_not_touch_another_machines_release() {
        assert!(matches!(
            authorize(&machine(), Some("me"), Some(&held_by("someone-else"))),
            Err(ApiError::ConflictReason("held_by_other_machine"))
        ));
        // An unresolved actor is never "the holder", whatever holds it.
        assert!(matches!(
            authorize(&machine(), None, Some(&held_by("someone-else"))),
            Err(ApiError::ConflictReason("held_by_other_machine"))
        ));
    }

    fn headers(name: header::HeaderName, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(name, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn preconditions_parse_strictly() {
        assert_eq!(parse_precondition(&HeaderMap::new()).unwrap(), None);
        assert_eq!(
            parse_precondition(&headers(header::IF_NONE_MATCH, "*")).unwrap(),
            Some(Expect::Absent)
        );
        assert_eq!(
            parse_precondition(&headers(header::IF_MATCH, "\"3\"")).unwrap(),
            Some(Expect::Version(3))
        );
        assert_eq!(
            parse_precondition(&headers(header::IF_MATCH, "3")).unwrap(),
            Some(Expect::Version(3))
        );
        for bad in ["W/\"3\"", "\"3\", \"4\"", "three", ""] {
            assert!(
                matches!(
                    parse_precondition(&headers(header::IF_MATCH, bad)),
                    Err(ApiError::BadRequest)
                ),
                "{bad:?} must be refused"
            );
        }
        assert!(matches!(
            parse_precondition(&headers(header::IF_NONE_MATCH, "\"3\"")),
            Err(ApiError::BadRequest)
        ));
    }

    #[test]
    fn a_machine_must_send_a_precondition_and_a_person_need_not() {
        assert!(matches!(
            precondition(&HeaderMap::new(), &machine()),
            Err(ApiError::PreconditionRequired)
        ));
        assert!(matches!(
            precondition(&HeaderMap::new(), &key()),
            Err(ApiError::PreconditionRequired)
        ));
        assert_eq!(
            precondition(&HeaderMap::new(), &principal_for("u")).unwrap(),
            None
        );
    }
}
