fn main() {
    // Run `serve()` on the server only
    #[cfg(feature = "server")]
    dioxus::serve(|| async move {
        // Create a new router for our app using the `router` function
        let router = dioxus::server::router(sample_service::App);
        let openapi = sample_service::sample_api_router(sample_service::AppState {});

        // .. customize the router, adding layers and new routes

        // And then return the router
        Ok(router.merge(openapi))
    });

    // When not on the server, just run `launch()` like normal
    #[cfg(not(feature = "server"))]
    dioxus::launch(sample_service::App);
}
