//! OpenID Connect sign-in for the server image: Pocket ID, Authentik,
//! Keycloak, anything that speaks OIDC discovery.
//!
//! The standard confidential-client flow, nothing exotic:
//!
//!   1. `/auth/login` → the provider's authorization endpoint, with a random
//!      `state` (CSRF), a random `nonce` (token replay) and a PKCE challenge;
//!   2. the provider sends the browser back to `/auth/callback?code&state`;
//!   3. the server exchanges the code at the token endpoint (client secret +
//!      PKCE verifier) and **verifies the ID token**: signature against the
//!      provider's JWKS, asymmetric algorithms only, `iss`, `aud`, `exp`,
//!      `nonce`;
//!   4. if the identity is allowed (see [`OidcConfig::allows`]) a signed
//!      session cookie is set (`super::session`).
//!
//! Discovery is fetched lazily and kept: a provider that is down when Ranch
//! starts does not stop it from starting, it stops people from signing in
//! until it is back. An unknown `kid` refreshes the keys once (key rotation).

use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

use super::config::OidcConfig;

/// Who the provider says this is.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub sub: String,
    pub name: Option<String>,
    pub email: Option<String>,
    /// The provider says it verified `email` (`email_verified`). An address a
    /// user could type into their profile is no proof of who they are.
    pub email_verified: bool,
    pub groups: Vec<String>,
}

#[derive(Debug)]
pub enum OidcError {
    /// The provider could not be reached or answered nonsense: not the user's fault.
    Provider(String),
    /// The provider answered, and what it said does not check out.
    Rejected(String),
    /// A valid identity that is not on the allow-list.
    NotAllowed(String),
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(m) | Self::Rejected(m) | Self::NotAllowed(m) => f.write_str(m),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Vec<String>,
}

struct Provider {
    discovery: Discovery,
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    name: Option<String>,
    preferred_username: Option<String>,
    email: Option<String>,
    /// A boolean, or the string "true" from providers that send it so.
    #[serde(default)]
    email_verified: Option<serde_json::Value>,
    #[serde(default)]
    groups: Vec<String>,
}

pub struct OidcClient {
    config: OidcConfig,
    http: reqwest::Client,
    provider: RwLock<Option<Arc<Provider>>>,
}

impl OidcClient {
    pub fn new(config: OidcConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            // A redirect from the token endpoint is never legitimate.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a default reqwest client");
        Self {
            config,
            http,
            provider: RwLock::new(None),
        }
    }

    pub fn config(&self) -> &OidcConfig {
        &self.config
    }

    async fn provider(&self, refresh: bool) -> Result<Arc<Provider>, OidcError> {
        if !refresh {
            if let Some(p) = self.provider.read().await.as_ref() {
                return Ok(p.clone());
            }
        }
        let url = format!("{}/.well-known/openid-configuration", self.config.issuer);
        let discovery: Discovery = self.get_json(&url).await?;
        if discovery.issuer.trim_end_matches('/') != self.config.issuer {
            return Err(OidcError::Provider(format!(
                "{url} describes issuer {:?}, not {:?}: check RANCH_OIDC_ISSUER",
                discovery.issuer, self.config.issuer
            )));
        }
        // Keys this build cannot use (an unknown curve…) are skipped, not fatal.
        let jwks: serde_json::Value = self.get_json(&discovery.jwks_uri).await?;
        let keys: Vec<Jwk> = jwks
            .get("keys")
            .and_then(|k| k.as_array())
            .map(|keys| {
                keys.iter()
                    .filter_map(|k| serde_json::from_value(k.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        if keys.is_empty() {
            return Err(OidcError::Provider(format!(
                "{} lists no usable signing key",
                discovery.jwks_uri
            )));
        }
        let provider = Arc::new(Provider { discovery, keys });
        *self.provider.write().await = Some(provider.clone());
        Ok(provider)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, OidcError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("cannot reach {url}: {e}")))?;
        if !response.status().is_success() {
            return Err(OidcError::Provider(format!(
                "{url} answered {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("{url} is not what was expected: {e}")))
    }

    /// Where to send the browser to sign in.
    pub async fn authorization_url(
        &self,
        state: &str,
        nonce: &str,
        challenge: &str,
    ) -> Result<String, OidcError> {
        let provider = self.provider(false).await?;
        let mut url = reqwest::Url::parse(&provider.discovery.authorization_endpoint)
            .map_err(|e| OidcError::Provider(format!("bad authorization endpoint: {e}")))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_url)
            .append_pair("scope", &self.config.scopes)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }

    /// The code the provider handed the browser → a verified, allowed identity.
    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Identity, OidcError> {
        let provider = self.provider(false).await?;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.config.redirect_url.as_str()),
            ("code_verifier", verifier),
        ];
        let methods = &provider.discovery.token_endpoint_auth_methods_supported;
        // `client_secret_basic` is the default of the spec; `post` when it is all there is.
        let basic = methods.is_empty() || methods.iter().any(|m| m == "client_secret_basic");
        let mut request = self.http.post(&provider.discovery.token_endpoint);
        if basic {
            request = request.basic_auth(&self.config.client_id, Some(&self.config.client_secret));
        } else {
            form.push(("client_id", self.config.client_id.as_str()));
            form.push(("client_secret", self.config.client_secret.as_str()));
        }
        let response =
            request.form(&form).send().await.map_err(|e| {
                OidcError::Provider(format!("cannot reach the token endpoint: {e}"))
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(OidcError::Rejected(format!(
                "the provider refused the sign-in code ({status}): {}",
                detail.chars().take(200).collect::<String>()
            )));
        }
        let tokens: TokenResponse = response
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("unreadable token response: {e}")))?;
        let id_token = tokens.id_token.ok_or_else(|| {
            OidcError::Rejected(
                "the provider returned no id_token; is `openid` in RANCH_OIDC_SCOPES?".to_string(),
            )
        })?;

        let claims = match self.verify(&provider, &id_token) {
            Ok(claims) => claims,
            // Signed with a key we have not seen: the provider may have rotated.
            Err(VerifyError::UnknownKey) => {
                let refreshed = self.provider(true).await?;
                self.verify(&refreshed, &id_token)
                    .map_err(|e| OidcError::Rejected(e.to_string()))?
            }
            Err(e) => return Err(OidcError::Rejected(e.to_string())),
        };
        if claims.nonce.as_deref() != Some(nonce) {
            return Err(OidcError::Rejected(
                "the ID token does not answer this sign-in (nonce mismatch)".to_string(),
            ));
        }

        let email_verified = matches!(&claims.email_verified, Some(serde_json::Value::Bool(true)))
            || matches!(&claims.email_verified, Some(serde_json::Value::String(s)) if s == "true");
        let identity = Identity {
            sub: claims.sub,
            name: claims.name.or(claims.preferred_username),
            email: claims.email,
            email_verified,
            groups: claims.groups,
        };
        self.config
            .allows(&identity)
            .map_err(OidcError::NotAllowed)?;
        Ok(identity)
    }

    fn verify(&self, provider: &Provider, id_token: &str) -> Result<IdClaims, VerifyError> {
        let header = decode_header(id_token).map_err(|e| VerifyError::Invalid(e.to_string()))?;
        // Asymmetric only: an `HS256` token "signed" with the public key is the
        // oldest trick there is.
        if matches!(
            header.alg,
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
        ) {
            return Err(VerifyError::Invalid(format!(
                "ID token signed with {:?}: only asymmetric algorithms are accepted",
                header.alg
            )));
        }
        let key = match &header.kid {
            Some(kid) => provider
                .keys
                .iter()
                .find(|k| k.common.key_id.as_deref() == Some(kid)),
            None if provider.keys.len() == 1 => provider.keys.first(),
            None => None,
        }
        .ok_or(VerifyError::UnknownKey)?;
        let decoding_key =
            DecodingKey::from_jwk(key).map_err(|e| VerifyError::Invalid(e.to_string()))?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&self.config.issuer, &format!("{}/", self.config.issuer)]);
        validation.set_audience(&[&self.config.client_id]);
        validation.leeway = 60;
        decode::<IdClaims>(id_token, &decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| VerifyError::Invalid(format!("the ID token does not verify: {e}")))
    }
}

enum VerifyError {
    UnknownKey,
    Invalid(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey => {
                f.write_str("the ID token is signed with a key the provider does not publish")
            }
            Self::Invalid(m) => f.write_str(m),
        }
    }
}

impl OidcConfig {
    /// With no allow-list, the provider decides (Pocket ID restricts a client
    /// to user groups itself). With lists, matching **either** is enough.
    pub fn allows(&self, identity: &Identity) -> Result<(), String> {
        if self.allowed_emails.is_empty() && self.allowed_groups.is_empty() {
            return Ok(());
        }
        // Only a verified address counts: on a provider where users edit
        // their own email, an unverified one could be anyone's.
        let email_ok = identity.email_verified
            && identity
                .email
                .as_ref()
                .is_some_and(|e| self.allowed_emails.contains(&e.to_lowercase()));
        let group_ok = identity
            .groups
            .iter()
            .any(|g| self.allowed_groups.contains(g));
        if email_ok || group_ok {
            Ok(())
        } else {
            Err(format!(
                "{} is signed in, but is not allowed to use Ranch",
                identity
                    .email
                    .clone()
                    .or_else(|| identity.name.clone())
                    .unwrap_or_else(|| identity.sub.clone())
            ))
        }
    }
}
