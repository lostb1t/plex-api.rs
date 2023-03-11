pub mod account;
pub mod claim_token;
pub mod device;
pub mod pin;
pub mod privacy;
pub mod sharing;
pub mod webhook;

use self::account::MyPlexAccount;
use self::claim_token::ClaimToken;
use self::device::DeviceManager;
use self::pin::PinManager;
use self::privacy::Privacy;
use self::sharing::Sharing;
use self::webhook::WebhookManager;
use crate::http_client::{HttpClient, HttpClientBuilder};
use crate::media_container::server::Feature;
use crate::url::{MYPLEX_SIGNIN_PATH, MYPLEX_SIGNOUT_PATH, MYPLEX_USER_INFO_PATH};
use crate::{Error, Result, Server};
use http::StatusCode;
use isahc::{AsyncBody, AsyncReadResponseExt, Response as HttpResponse};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct MyPlex {
    client: Arc<HttpClient>,
    account: Option<MyPlexAccount>,
}

impl MyPlex {
    pub fn new(client: HttpClient) -> Result<Self> {
        if !client.is_authenticated() {
            return Err(Error::ClientNotAuthenticated);
        }

        Ok(Self {
            client: Arc::new(client),
            account: None,
        })
    }

    async fn login_internal(
        username: &str,
        password: &str,
        client: HttpClient,
        extra_params: &[(&str, &str)],
    ) -> Result<Self> {
        if client.is_authenticated() {
            return Err(Error::ClientAuthenticated);
        }

        let mut params = vec![
            ("login", username),
            ("password", password),
            ("rememberMe", "true"),
        ];
        for (key, value) in extra_params {
            params.push((key, value));
        }

        let response = client
            .post(MYPLEX_SIGNIN_PATH)
            .header("Accept", "application/json")
            .form(&params)?
            .send()
            .await?;

        Self::build_from_signin_response(client, response).await
    }

    async fn login(username: &str, password: &str, client: HttpClient) -> Result<Self> {
        Self::login_internal(username, password, client, &[]).await
    }

    async fn login_with_otp(
        username: &str,
        password: &str,
        verification_code: &str,
        client: HttpClient,
    ) -> Result<Self> {
        Self::login_internal(
            username,
            password,
            client,
            &[("verificationCode", verification_code)],
        )
        .await
    }

    pub async fn refresh(self) -> Result<Self> {
        let response = self
            .client
            .get(MYPLEX_USER_INFO_PATH)
            .header("Accept", "application/json")
            .body(())?
            .send()
            .await?;

        Self::build_from_signin_response(self.client.as_ref().to_owned(), response).await
    }

    async fn build_from_signin_response(
        client: HttpClient,
        mut response: HttpResponse<AsyncBody>,
    ) -> Result<Self> {
        match response.status() {
            StatusCode::OK | StatusCode::CREATED => {
                let account: account::MyPlexAccount = response.json().await?;
                Ok(Self {
                    client: Arc::new(client.set_x_plex_token(account.auth_token.clone())),
                    account: Some(account),
                })
            }
            _ => Err(Error::from_response(response).await),
        }
    }

    pub fn client(&self) -> &HttpClient {
        &self.client
    }

    /// Get a claim token from the API, which can be used for attaching a server to your account.
    /// See <https://hub.docker.com/r/plexinc/pms-docker> for details, look for "PLEX_CLAIM".
    pub async fn claim_token(&self) -> Result<ClaimToken> {
        ClaimToken::new(&self.client).await
    }

    /// Get privacy settings for your account. You can update the settings using the returned object.
    /// See [Privacy Preferences on plex.tv](https://www.plex.tv/about/privacy-legal/privacy-preferences/#opd) for details.
    pub async fn privacy(&self) -> Result<Privacy> {
        Privacy::new(self.client.clone()).await
    }

    pub fn sharing(&self) -> Sharing {
        Sharing::new(self.client.clone())
    }

    pub fn available_features(&self) -> Option<&Vec<Feature>> {
        return self
            .account
            .as_ref()
            .map(|account| &account.subscription.features);
    }

    pub fn account(&self) -> Option<&MyPlexAccount> {
        return self.account.as_ref();
    }

    pub async fn webhook_manager(&self) -> Result<WebhookManager> {
        if let Some(features) = self.available_features() {
            if !features.contains(&Feature::Webhooks) {
                return Err(Error::SubscriptionFeatureNotAvailable(Feature::Webhooks));
            }
        }
        WebhookManager::new(self.client.clone()).await
    }

    pub fn device_manager(&self) -> DeviceManager {
        DeviceManager::new(self.client.clone())
    }

    pub fn pin_manager(&self) -> PinManager {
        PinManager::new(self.client.clone())
    }

    /// Sign out of your account. It's highly recommended to call this method when you're done using the API.
    /// At least when you obtained the MyPlex instance using [MyPlex::login](struct.MyPlex.html#method.login).
    pub async fn signout(self) -> Result {
        let response = self
            .client
            .delete(MYPLEX_SIGNOUT_PATH)
            .body(())?
            .send()
            .await?;

        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            _ => Err(Error::from_response(response).await),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MyPlexBuilder<'a> {
    client: Option<HttpClient>,
    token: Option<SecretString>,
    username: Option<&'a str>,
    password: Option<SecretString>,
    otp: Option<SecretString>,
    test_token_auth: bool,
}

impl<'a> Default for MyPlexBuilder<'a> {
    fn default() -> Self {
        Self {
            client: None,
            token: None,
            username: None,
            password: None,
            otp: None,
            test_token_auth: true,
        }
    }
}

impl MyPlexBuilder<'_> {
    pub fn set_client(self, client: HttpClient) -> Self {
        Self {
            client: Some(client),
            token: self.token,
            username: self.username,
            password: self.password,
            otp: self.otp,
            test_token_auth: self.test_token_auth,
        }
    }

    pub fn set_test_token_auth(self, test_token_auth: bool) -> Self {
        Self {
            client: self.client,
            token: self.token,
            username: self.username,
            password: self.password,
            otp: self.otp,
            test_token_auth,
        }
    }

    pub async fn build(self) -> Result<MyPlex> {
        let mut client = if let Some(client) = self.client {
            client
        } else {
            HttpClientBuilder::default().build()?
        };

        if let (Some(username), Some(password)) = (self.username, self.password) {
            if let Some(otp) = self.otp {
                return MyPlex::login_with_otp(
                    username,
                    password.expose_secret(),
                    otp.expose_secret(),
                    client,
                )
                .await;
            } else {
                return MyPlex::login(username, password.expose_secret(), client).await;
            }
        }

        if self.otp.is_some() {
            return Err(Error::UselessOtp);
        }

        if let Some(token) = self.token {
            client = client.set_x_plex_token(token);
        }

        let mut plex_result = MyPlex::new(client);

        if self.test_token_auth {
            if let Ok(plex) = plex_result {
                plex_result = plex.refresh().await;
            }
        }

        plex_result
    }
}

impl<'a> MyPlexBuilder<'a> {
    pub fn set_token<T>(self, token: T) -> Self
    where
        T: Into<SecretString>,
    {
        Self {
            client: self.client,
            token: Some(token.into()),
            username: self.username,
            password: self.password,
            otp: self.otp,
            test_token_auth: self.test_token_auth,
        }
    }

    pub fn set_username_and_password<T>(self, username: &'a str, password: T) -> Self
    where
        T: Into<SecretString>,
    {
        Self {
            client: self.client,
            token: self.token,
            username: Some(username),
            password: Some(password.into()),
            otp: self.otp,
            test_token_auth: self.test_token_auth,
        }
    }

    pub fn set_otp<T>(self, otp: T) -> Self
    where
        T: Into<SecretString>,
    {
        Self {
            client: self.client,
            token: self.token,
            username: self.username,
            password: self.password,
            otp: Some(otp.into()),
            test_token_auth: self.test_token_auth,
        }
    }
}
