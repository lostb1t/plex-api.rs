#[cfg(feature = "test_connect_authenticated")]
#[tokio::test]
async fn decode_resources_online() {
    use crate::MyPlexAccount;
    use std::env;
    let acc: Result<MyPlexAccount, _> = {
        let auth_token = env::var("PLEX_API_AUTH_TOKEN").expect("Auth token not specified");
        MyPlexAccount::by_token(&auth_token).await
    };
    assert!(acc.is_ok(), "Unable to authenticate");
    let resources = acc.unwrap().get_resources().await;
    assert!(
        resources.is_ok(),
        "Unable to get resources: {:?}",
        resources.err()
    );
}
