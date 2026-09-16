//! Pagination, clamping, and ordering coverage for `get_samples`.
//!
//! SQLite tests assert exact counts against a fresh database; Postgres tests
//! run in an isolated database per case, so exact counts hold there too.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod support;

use api::{CreateSampleResponse, GetSamplesServerResponse, NewSample, SampleApi, Status};

async fn seed(state: &api::AppState, prefix: &str, count: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for index in 0..count {
        let name = format!("{prefix}-{index}");
        let sample = support::full_sample(&name);
        let CreateSampleResponse::Created(created) = state.create_sample(sample).await;
        let data = created.data.expect("create returns data");
        ids.push(data.id);
    }
    ids
}

async fn list(
    state: &api::AppState,
    offset: Option<i64>,
    limit: Option<i64>,
    backend: &str,
) -> Vec<api::Sample> {
    let GetSamplesServerResponse::Ok(body) = state.get_samples(offset, limit).await;
    assert!(body.errors.is_empty(), "{backend}: no errors");
    body.data
}

#[tokio::test]
async fn limit_slices_from_the_start_in_order() {
    crate::on_both_backends!("page_slice", |state| {
        let backend = state.backend_name();
        let ids = seed(state, "slice", 5).await;
        let rows = list(state, None, Some(2), backend).await;
        assert_eq!(rows.len(), 2, "{backend}");
        let first_id = rows.first().expect("row").id.clone();
        assert_eq!(first_id, ids.first().expect("id").to_string(), "{backend}");
    });
}

#[tokio::test]
async fn offset_skips_rows() {
    crate::on_both_backends!("page_offset", |state| {
        let backend = state.backend_name();
        let ids = seed(state, "skip", 5).await;
        let rows = list(state, Some(2), Some(2), backend).await;
        assert_eq!(rows.len(), 2, "{backend}");
        let first_id = rows.first().expect("row").id.clone();
        assert_eq!(first_id, ids.get(2).expect("id").to_string(), "{backend}");
    });
}

#[tokio::test]
async fn offset_past_the_end_returns_empty_without_errors() {
    crate::on_both_backends!("page_empty", |state| {
        let backend = state.backend_name();
        seed(state, "tail", 3).await;
        let rows = list(state, Some(99), Some(10), backend).await;
        assert!(rows.is_empty(), "{backend}: empty page");
    });
}

#[tokio::test]
async fn negative_offset_clamps_to_zero() {
    crate::on_both_backends!("page_neg_off", |state| {
        let backend = state.backend_name();
        seed(state, "neg", 3).await;
        let clamped = list(state, Some(-5), Some(10), backend).await;
        let from_zero = list(state, Some(0), Some(10), backend).await;
        assert_eq!(clamped.len(), from_zero.len(), "{backend}");
        let clamped_id = clamped.first().expect("row").id.clone();
        let zero_id = from_zero.first().expect("row").id.clone();
        assert_eq!(clamped_id, zero_id, "{backend}");
    });
}

#[tokio::test]
async fn non_positive_limit_clamps_to_one() {
    crate::on_both_backends!("page_min_lim", |state| {
        let backend = state.backend_name();
        seed(state, "minlim", 3).await;
        for limit in [0, -10] {
            let rows = list(state, None, Some(limit), backend).await;
            assert_eq!(rows.len(), 1, "{backend}: limit {limit} clamps to 1");
        }
    });
}

#[tokio::test]
async fn huge_limit_returns_everything() {
    crate::on_both_backends!("page_max_lim", |state| {
        let backend = state.backend_name();
        seed(state, "big", 4).await;
        let rows = list(state, None, Some(5000), backend).await;
        assert_eq!(rows.len(), 4, "{backend}: clamped to 1000, returns all");
    });
}

#[tokio::test]
async fn defaults_return_everything_in_creation_order() {
    crate::on_both_backends!("page_default", |state| {
        let backend = state.backend_name();
        let ids = seed(state, "ordered", 6).await;
        let rows = list(state, None, None, backend).await;
        let listed: Vec<String> = rows.into_iter().map(|row| row.id).collect();
        assert_eq!(listed, ids, "{backend}: ascending creation order");
    });
}

#[tokio::test]
async fn names_with_varied_statuses_all_list() {
    crate::on_both_backends!("page_mixed", |state| {
        let backend = state.backend_name();
        for (index, status) in [Status::Active, Status::Inactive, Status::Archived]
            .into_iter()
            .enumerate()
        {
            state
                .create_sample(NewSample::new(format!("mix-{index}"), status))
                .await;
        }
        let rows = list(state, None, None, backend).await;
        assert_eq!(rows.len(), 3, "{backend}");
    });
}
