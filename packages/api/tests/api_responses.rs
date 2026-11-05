//! Response-shape and error-payload coverage for every operation.
//!
//! Each case runs against SQLite always and an isolated Postgres database
//! when a server is reachable. The suite locks in the API contract: success
//! payloads carry `data` with empty `errors` (HTTP 200/201), while
//! missing/invalid ids return `data: None` with exactly one `NOT_FOUND`
//! error and an HTTP 404 transport status — see `tests/http.rs` for the
//! over-the-wire assertions.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod support;

use api::{
    CodegenApi, CreateSampleResponse, Error, ErrorCode, GetSampleServerResponse, NewSample,
    RemoveSampleServerResponse, SampleUpdate, Status, UpdateSampleServerResponse,
};

fn assert_not_found(errors: &[Error], sample_id: &str, backend: &str) {
    assert_eq!(errors.len(), 1, "{backend}: exactly one error");
    let error = errors.first().expect("one error");
    assert_eq!(error.code, ErrorCode::NotFound, "{backend}");
    assert_eq!(error.status, 404, "{backend}");
    assert_eq!(error.title, "Not Found", "{backend}");
    assert!(
        error
            .message
            .as_ref()
            .expect("error has message")
            .contains(sample_id),
        "{backend}: detail mentions the id"
    );
    uuid::Uuid::parse_str(&error.id).expect("error id parses as a uuid");
}

async fn create_named(state: &api::AppState, name: &str, backend: &str) -> api::Sample {
    let body = support::full_sample(name);
    let CreateSampleResponse::Created(created) = state.create_sample(body).await;
    assert!(created.errors.is_empty(), "{backend}: no errors");
    created.data.expect("create returns data")
}

#[tokio::test]
async fn create_returns_all_fields() {
    crate::on_all_backends!("api_create", |state| {
        let backend = state.backend_name();
        let sample = create_named(state, "full", backend).await;
        assert_eq!(sample.name, "full");
        assert_eq!(sample.description, Some("full description".to_string()));
        assert_eq!(sample.status, Status::Inactive);
        assert_eq!(
            sample.tags,
            Some(vec!["one".to_string(), "two".to_string()])
        );
        assert_eq!(sample.tags_condensed, Some("one, two".to_string()));
        assert_eq!(
            sample.image_uri,
            Some("https://example.com/img.png".to_string())
        );
        assert_eq!(sample.updated_at, None);
        uuid::Uuid::parse_str(&sample.id).expect("id parses as a uuid");
    });
}

#[tokio::test]
async fn create_minimal_leaves_optionals_empty() {
    crate::on_all_backends!("api_create_min", |state| {
        let backend = state.backend_name();
        let CreateSampleResponse::Created(created) = state
            .create_sample(NewSample::new("minimal".to_string(), Status::Active))
            .await;
        assert!(created.errors.is_empty(), "{backend}: no errors");
        let sample = created.data.expect("create returns data");
        assert_eq!(sample.description, None);
        assert_eq!(sample.tags, None);
        assert_eq!(sample.tags_condensed, None);
        assert_eq!(sample.image_uri, None);
        assert_eq!(sample.updated_at, None);
    });
}

#[tokio::test]
async fn create_accepts_every_status() {
    crate::on_all_backends!("api_status", |state| {
        let backend = state.backend_name();
        for status in [Status::Active, Status::Inactive, Status::Archived] {
            let CreateSampleResponse::Created(created) = state
                .create_sample(NewSample::new("s".to_string(), status.clone()))
                .await;
            assert!(created.errors.is_empty(), "{backend}: no errors");
            assert_eq!(created.data.expect("data").status, status, "{backend}");
        }
    });
}

#[tokio::test]
async fn create_generates_unique_ids() {
    crate::on_all_backends!("api_unique", |state| {
        let backend = state.backend_name();
        let first = create_named(state, "u1", backend).await;
        let second = create_named(state, "u2", backend).await;
        assert_ne!(first.id, second.id, "{backend}: ids differ");
    });
}

#[tokio::test]
async fn get_rejects_malformed_ids() {
    crate::on_all_backends!("api_bad_id", |state| {
        let backend = state.backend_name();
        for bad_id in [
            "",
            "not-a-uuid",
            "123",
            "too short",
            "gggggggg-gggg-gggg-gggg-gggggggggggg",
            "0196a8e0-9e6f-7c9e-8000-000000000000-extra",
        ] {
            let GetSampleServerResponse::NotFound(body) =
                state.get_sample(bad_id.to_string()).await
            else {
                panic!("{backend}: expected HTTP 404")
            };
            assert!(body.data.is_none(), "{backend}: no data for {bad_id:?}");
            assert_not_found(&body.errors, bad_id, backend);
        }
    });
}

#[tokio::test]
async fn get_missing_uuid_returns_404_payload() {
    crate::on_all_backends!("api_missing", |state| {
        let backend = state.backend_name();
        let missing = uuid::Uuid::now_v7().to_string();
        let GetSampleServerResponse::NotFound(body) = state.get_sample(missing.clone()).await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        assert!(body.data.is_none(), "{backend}: no data");
        assert_not_found(&body.errors, &missing, backend);
    });
}

#[tokio::test]
async fn update_missing_and_malformed_return_404() {
    crate::on_all_backends!("api_upd_404", |state| {
        let backend = state.backend_name();
        let missing = uuid::Uuid::now_v7().to_string();
        let UpdateSampleServerResponse::NotFound(body) = state
            .update_sample(
                missing.clone(),
                SampleUpdate {
                    name: Some("ghost".to_string()),
                    ..Default::default()
                },
            )
            .await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        assert!(body.data.is_none(), "{backend}: no data");
        assert_not_found(&body.errors, &missing, backend);

        let UpdateSampleServerResponse::NotFound(bad) = state
            .update_sample(
                "junk".to_string(),
                SampleUpdate {
                    name: Some("ghost".to_string()),
                    ..Default::default()
                },
            )
            .await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        assert!(bad.data.is_none(), "{backend}: no data");
        assert_not_found(&bad.errors, "junk", backend);
    });
}

#[tokio::test]
async fn remove_missing_and_malformed_return_404() {
    crate::on_all_backends!("api_del_404", |state| {
        let backend = state.backend_name();
        let missing = uuid::Uuid::now_v7().to_string();
        let RemoveSampleServerResponse::NotFound(body) = state.remove_sample(missing.clone()).await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        assert!(body.data.is_none(), "{backend}: no data");
        assert_not_found(&body.errors, &missing, backend);

        let RemoveSampleServerResponse::NotFound(bad) = state.remove_sample(String::new()).await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        assert!(bad.data.is_none(), "{backend}: no data");
        assert_not_found(&bad.errors, "", backend);
    });
}

#[tokio::test]
async fn double_remove_second_is_404() {
    crate::on_all_backends!("api_double_del", |state| {
        let backend = state.backend_name();
        let created = create_named(state, "twice", backend).await;
        let RemoveSampleServerResponse::Ok(first) = state.remove_sample(created.id.clone()).await
        else {
            panic!("{backend}: expected HTTP 200")
        };
        assert!(first.errors.is_empty(), "{backend}: no errors");
        assert_eq!(
            first.data.expect("deleted row returned").id,
            created.id,
            "{backend}"
        );
        let RemoveSampleServerResponse::NotFound(second) =
            state.remove_sample(created.id.clone()).await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        assert!(second.data.is_none(), "{backend}: already gone");
        assert_not_found(&second.errors, &created.id, backend);
    });
}

#[tokio::test]
async fn remove_returns_the_deleted_row() {
    crate::on_all_backends!("api_del_row", |state| {
        let backend = state.backend_name();
        let created = create_named(state, "doomed", backend).await;
        let RemoveSampleServerResponse::Ok(removed) = state.remove_sample(created.id.clone()).await
        else {
            panic!("{backend}: expected HTTP 200")
        };
        let row = removed.data.expect("deleted row returned");
        assert_eq!(row.id, created.id, "{backend}");
        assert_eq!(row.name, "doomed", "{backend}");
        assert_eq!(row.description, created.description, "{backend}");
    });
}

#[tokio::test]
async fn update_empty_body_preserves_row_and_stamps() {
    crate::on_all_backends!("api_upd_empty", |state| {
        let backend = state.backend_name();
        let created = create_named(state, "stable", backend).await;
        let UpdateSampleServerResponse::Ok(updated) = state
            .update_sample(created.id.clone(), SampleUpdate::default())
            .await
        else {
            panic!("{backend}: expected HTTP 200")
        };
        assert!(updated.errors.is_empty(), "{backend}: no errors");
        let row = updated.data.expect("updated row returned");
        assert_eq!(row.id, created.id, "{backend}");
        assert_eq!(row.name, created.name, "{backend}");
        assert_eq!(row.description, created.description, "{backend}");
        assert_eq!(row.status, created.status, "{backend}");
        assert_eq!(row.tags, created.tags, "{backend}");
        assert_eq!(row.image_uri, created.image_uri, "{backend}");
        assert_eq!(row.created_at, created.created_at, "{backend}");
        assert!(row.updated_at.is_some(), "{backend}: stamped");
    });
}

#[tokio::test]
async fn update_replaces_all_mutable_fields() {
    crate::on_all_backends!("api_upd_all", |state| {
        let backend = state.backend_name();
        let created = create_named(state, "mutable", backend).await;
        let UpdateSampleServerResponse::Ok(updated) = state
            .update_sample(
                created.id.clone(),
                SampleUpdate {
                    name: Some("changed".to_string()),
                    description: Some("changed desc".to_string()),
                    status: Some(Status::Archived),
                    tags: Some(vec!["solo".to_string()]),
                    image_uri: Some("https://example.com/changed.png".to_string()),
                },
            )
            .await
        else {
            panic!("{backend}: expected HTTP 200")
        };
        assert!(updated.errors.is_empty(), "{backend}: no errors");
        let row = updated.data.expect("updated row returned");
        assert_eq!(row.name, "changed", "{backend}");
        assert_eq!(
            row.description,
            Some("changed desc".to_string()),
            "{backend}"
        );
        assert_eq!(row.status, Status::Archived, "{backend}");
        assert_eq!(row.tags, Some(vec!["solo".to_string()]), "{backend}");
        assert_eq!(row.tags_condensed, Some("solo".to_string()), "{backend}");
        assert_eq!(
            row.image_uri,
            Some("https://example.com/changed.png".to_string()),
            "{backend}"
        );
        // A follow-up read sees the same values (update is durable).
        let GetSampleServerResponse::Ok(refetched) = state.get_sample(created.id.clone()).await
        else {
            panic!("{backend}: expected HTTP 200")
        };
        let reread = refetched.data.expect("row persisted");
        assert_eq!(reread.name, "changed", "{backend}");
        assert_eq!(reread.status, Status::Archived, "{backend}");
    });
}

#[tokio::test]
async fn updated_at_never_predates_created_at() {
    crate::on_all_backends!("api_stamps", |state| {
        let backend = state.backend_name();
        let created = create_named(state, "stamped", backend).await;
        assert_eq!(created.updated_at, None, "{backend}: untouched");
        let UpdateSampleServerResponse::Ok(updated) = state
            .update_sample(
                created.id.clone(),
                SampleUpdate {
                    name: Some("stamped-2".to_string()),
                    ..Default::default()
                },
            )
            .await
        else {
            panic!("{backend}: expected HTTP 200")
        };
        let row = updated.data.expect("updated row returned");
        let stamped = row.updated_at.expect("stamped after update");
        assert!(stamped >= row.created_at, "{backend}: monotonic stamps");
        assert_eq!(row.created_at, created.created_at, "{backend}");
    });
}

#[tokio::test]
async fn error_ids_are_unique_valid_uuids() {
    crate::on_all_backends!("api_err_ids", |state| {
        let backend = state.backend_name();
        let first_missing = uuid::Uuid::now_v7().to_string();
        let second_missing = uuid::Uuid::now_v7().to_string();
        let GetSampleServerResponse::NotFound(first) = state.get_sample(first_missing).await else {
            panic!("{backend}: expected HTTP 404")
        };
        let GetSampleServerResponse::NotFound(second) = state.get_sample(second_missing).await
        else {
            panic!("{backend}: expected HTTP 404")
        };
        let first_id = first.errors.first().expect("error").id.clone();
        let second_id = second.errors.first().expect("error").id.clone();
        uuid::Uuid::parse_str(&first_id).expect("valid uuid");
        uuid::Uuid::parse_str(&second_id).expect("valid uuid");
        assert_ne!(first_id, second_id, "{backend}: error ids differ");
    });
}
