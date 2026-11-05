// @generated automatically by Diesel CLI.

diesel::table! {
    passkeys (id) {
        id -> Bytea,
        user_id -> Bytea,
        credential_id -> Bytea,
        passkey -> Text,
        created_at -> Timestamp,
        last_used_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    samples (id) {
        id -> Bytea,
        name -> Text,
        description -> Nullable<Text>,
        status -> Text,
        tags -> Nullable<Text>,
        image_uri -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    users (id) {
        id -> Bytea,
        username -> Text,
        password_hash -> Text,
        created_at -> Timestamp,
    }
}

diesel::joinable!(passkeys -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(passkeys, samples, users,);
