//! Concurrency coverage: `AppState` is shared across tasks.
//!
//! The single `Mutex` connection serializes queries; these tests prove cloned
//! handles can be used from many tasks at once without errors, lost writes,
//! or duplicate ids.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod support;

use api::{CodegenApi, CreateSampleResponse, GetSamplesServerResponse};
use std::collections::HashSet;

async fn concurrent_creates(
    state: &api::AppState,
    prefix: &str,
    tasks: usize,
    backend: &str,
) -> Vec<String> {
    let mut handles = Vec::new();
    for index in 0..tasks {
        let state = state.clone();
        let name = format!("{prefix}-{index}");
        handles.push(tokio::spawn(async move {
            state.create_sample(support::full_sample(&name)).await
        }));
    }
    let mut ids = Vec::new();
    for handle in handles {
        let CreateSampleResponse::Created(created) = handle.await.expect("task joins");
        assert!(
            created.errors.is_empty(),
            "{backend}: no errors under concurrency"
        );
        ids.push(created.data.expect("create returns data").id);
    }
    ids
}

#[tokio::test]
async fn parallel_creates_all_succeed_with_unique_ids() {
    crate::on_both_backends!("conc_create", |state| {
        let backend = state.backend_name();
        let ids = concurrent_creates(state, "conc", 10, backend).await;
        assert_eq!(ids.len(), 10, "{backend}");
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 10, "{backend}: ids unique");
        // Every write landed: the final list holds exactly what we created.
        let GetSamplesServerResponse::Ok(listed) = state.get_samples(None, None).await;
        assert!(listed.errors.is_empty(), "{backend}");
        assert_eq!(listed.data.len(), 10, "{backend}: nothing lost");
    });
}

#[tokio::test]
async fn parallel_reads_alongside_writes_stay_clean() {
    crate::on_both_backends!("conc_mixed", |state| {
        let backend = state.backend_name();
        let writer = {
            let state = state.clone();
            tokio::spawn(async move { concurrent_creates(&state, "mixed", 5, backend).await })
        };
        let mut readers = Vec::new();
        for _ in 0..5 {
            let state = state.clone();
            readers.push(tokio::spawn(async move {
                let GetSamplesServerResponse::Ok(listed) = state.get_samples(None, Some(100)).await;
                assert!(listed.errors.is_empty());
                listed.data.len()
            }));
        }
        let written = writer.await.expect("writer joins");
        assert_eq!(written.len(), 5, "{backend}");
        for reader in readers {
            reader.await.expect("reader joins");
        }
        let GetSamplesServerResponse::Ok(final_list) = state.get_samples(None, None).await;
        assert_eq!(final_list.data.len(), 5, "{backend}: all writes visible");
    });
}
