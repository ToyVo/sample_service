use crate::{
    CreateSampleResponse, CreateSampleResponse201, GetSampleResponse, GetSampleServerResponse,
    GetSamplesResponse, GetSamplesServerResponse, NewSample, RemoveSampleResponse,
    RemoveSampleServerResponse, SampleApi, SampleUpdate, UpdateSampleResponse,
    UpdateSampleServerResponse,
};

#[derive(Clone)]
pub struct AppState {/* state goes here */}

#[async_trait::async_trait]
impl SampleApi for AppState {
    async fn create_sample(&self, _body: NewSample) -> CreateSampleResponse {
        CreateSampleResponse::Created(CreateSampleResponse201 {
            data: None,
            errors: Vec::new(),
        })
    }

    async fn remove_sample(&self, _sample_id: String) -> RemoveSampleServerResponse {
        RemoveSampleServerResponse::Ok(RemoveSampleResponse {
            data: None,
            errors: Vec::new(),
        })
    }

    async fn update_sample(
        &self,
        _sample_id: String,
        _body: SampleUpdate,
    ) -> UpdateSampleServerResponse {
        UpdateSampleServerResponse::Ok(UpdateSampleResponse {
            data: None,
            errors: Vec::new(),
        })
    }

    async fn get_sample(&self, _sample_id: String) -> GetSampleServerResponse {
        GetSampleServerResponse::Ok(GetSampleResponse {
            data: None,
            errors: Vec::new(),
        })
    }

    async fn get_samples(
        &self,
        _offset: Option<i64>,
        _limit: Option<i64>,
    ) -> GetSamplesServerResponse {
        GetSamplesServerResponse::Ok(GetSamplesResponse {
            data: Vec::new(),
            errors: Vec::new(),
        })
    }
}
