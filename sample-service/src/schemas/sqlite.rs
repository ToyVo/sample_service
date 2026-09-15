// @generated automatically by Diesel CLI.

diesel::table! {
    samples (id) {
        id -> Binary,
        name -> Text,
        description -> Nullable<Text>,
        status -> Text,
        tags -> Nullable<Text>,
        image_uri -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}
