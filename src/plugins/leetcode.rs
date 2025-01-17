use self::req::{Json, Mode, Req};
use crate::{
    cfg::{self, Config},
    err::Error,
    plugins::chrome,
};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, ClientBuilder, Response,
};
use std::{collections::HashMap, str::FromStr, time::Duration};
use ::function_name::named;

/// LeetCode API set
#[derive(Clone)]
pub struct LeetCode {
    pub conf: Config,
    client: Client,
    default_headers: HeaderMap,
}

macro_rules! make_req {
    ($self:ident, $url:expr) => {
        Req::new($self.default_headers.to_owned(), function_name!(), $url)
    }
}

impl LeetCode {
    /// Parse reqwest headers
    fn headers(mut headers: HeaderMap, ts: Vec<(&str, &str)>) -> Result<HeaderMap, Error> {
        for (k, v) in ts.into_iter() {
            let name = HeaderName::from_str(k);
            let value = HeaderValue::from_str(v);
            if name.is_err() || value.is_err() {
                return Err(Error::ParseError("http header parse failed".to_string()));
            }

            headers.insert(name.unwrap(), value.unwrap());
        }

        Ok(headers)
    }

    /// New LeetCode client
    pub fn new() -> Result<LeetCode, crate::Error> {
        let conf = cfg::locate()?;
        let cookies = chrome::cookies()?;
        let default_headers = LeetCode::headers(
            HeaderMap::new(),
            vec![
                ("Cookie", cookies.to_string().as_str()),
                ("x-csrftoken", &cookies.csrf),
                ("x-requested-with", "XMLHttpRequest"),
                ("Origin", &conf.sys.urls["base"]),
            ],
        )?;

        let client = ClientBuilder::new()
            .gzip(true)
            .connect_timeout(Duration::from_secs(30))
            .build()?;

        // Sync conf
        if conf.cookies.csrf != cookies.csrf {
            conf.sync()?;
        }

        Ok(LeetCode {
            conf,
            client,
            default_headers,
        })
    }

    /// Generic GraphQL query
    #[named]
    pub async fn get_graphql(&self, query: String, variables: Option<String>) -> Result<Response, Error> {
        let url = &self.conf.sys.urls.get("graphql").ok_or(Error::NoneError)?;
        let refer = self.conf.sys.urls.get("base").ok_or(Error::NoneError)?;
        let mut json: Json = HashMap::new();
        json.insert("operationName", "a".to_string());
        if let Some(v) = variables {
            json.insert("variables", v);
        }
        json.insert("query", query);

        let mut req = make_req!(self, url.to_string());
        req.mode = Mode::Post(json);
        req.refer = Some(refer.to_string());
        req
        .send(&self.client)
        .await
    }

    /// Get category problems
    #[named]
    pub async fn get_category_problems(&self, category: &str) -> Result<Response, Error> {
        trace!("Requesting {} problems...", &category);
        let url = &self
            .conf
            .sys
            .urls
            .get("problems").ok_or(Error::NoneError)?
            .replace("$category", category);

        make_req!(self, url.to_string())
        .send(&self.client)
        .await
    }

    pub async fn get_question_ids_by_tag(&self, slug: &str) -> Result<Response, Error> {
        self.get_graphql("query a {
            topicTag(slug: \"$slug\") {
              questions {
               questionId
              }
            }
          }"
          .replace("$slug", slug),
        None).await
    }

    /// Get user info
    pub async fn get_user_info(&self) -> Result<Response, Error> {
        self.get_graphql("query a {
           user {
             username
             isCurrentUserPremium
           }
         }".to_owned(),
        None).await
    }

    /// Get daily problem
    pub async fn get_question_daily(&self) -> Result<Response, Error> {
        trace!("Requesting daily problem...");
        self.get_graphql(
          "query a {
             activeDailyCodingChallengeQuestion {
               question {
                 questionFrontendId
               }
             }
           }".to_owned(), None
        ).await
    }

    /// Register for a contest
    #[named]
    pub async fn register_contest(&self, contest: &str) -> Result<Response,Error> {
        let url = self.conf.sys.urls.get("contest_register")
            .ok_or(Error::NoneError)?
            .replace("$contest_slug", contest);
        let mut req = make_req!(self, url);
        req.mode = Mode::Post(HashMap::new());
        req
        .send(&self.client)
        .await
    }

    /// Get contest info
    #[named]
    pub async fn get_contest_info(&self, contest: &str) -> Result<Response, Error> {
        trace!("Requesting {} detail...", contest);
        // cannot use the graphql API here because it does not provide registration status
        let url = &self.conf.sys.urls
            .get("contest_info")
            .ok_or(Error::NoneError)?
            .replace("$contest_slug", contest);
        make_req!(self, url.to_string())
        .send(&self.client)
        .await
    }

    /// Get full question detail
    pub async fn get_question_detail(&self, problem: &str) -> Result<Response,Error> {
        self.get_graphql("query a($s: String!) {
           question(titleSlug: $s) {
             title
             titleSlug
             questionId
             questionFrontendId
             categoryTitle
             content
             codeDefinition
             status
             metaData
             codeSnippets {
               langSlug
               lang
               code
             }
             isPaidOnly
             exampleTestcases
             sampleTestCase
             enableRunCode
             stats
             translatedContent
             isFavor
             difficulty
           }
         }".to_owned(), Some(
            r#"{"s": "$s"}"#.replace("$s", problem)
        )).await
    }


    /// Send code to judge
    #[named]
    pub async fn run_code(&self, j: Json, url: String, refer: String) -> Result<Response, Error> {
        info!("Sending code to judge...");
        let mut req = make_req!(self, url);
        req.mode = Mode::Post(j);
        req.refer = Some(refer);
        req
        .send(&self.client)
        .await
    }

    /// Get the result of submission / testing
    #[named]
    pub async fn verify_result(&self, id: String) -> Result<Response, Error> {
        let url = self.conf.sys.urls.get("verify").ok_or(Error::NoneError)?.replace("$id", &id);
        make_req!(self, url)
        .send(&self.client)
        .await
    }
}

/// Sub-module for leetcode, simplify requests
mod req {
    use super::LeetCode;
    use crate::err::Error;
    use reqwest::{header::HeaderMap, Client, Response};
    use std::collections::HashMap;
    use derive_new::new;

    /// Standardize json format
    pub type Json = HashMap<&'static str, String>;

    /// Standardize request mode
    pub enum Mode {
        Get,
        Post(Json),
    }

    /// LeetCode request prototype
    #[derive(new)]
    pub struct Req {
        pub default_headers: HeaderMap,
        pub name: &'static str,
        pub url: String,
        #[new(value = "Mode::Get")]
        pub mode: Mode,
        #[new(default)]
        pub refer: Option<String>,
    }

    impl Req {
        pub async fn send(self, client: &Client) -> Result<Response, Error> {
            trace!("Running leetcode::{}...", &self.name);
            let url = self.url.to_owned();
            let headers = LeetCode::headers(
                self.default_headers,
                vec![("Referer", &self.refer.unwrap_or(url))],
            )?;

            let req = match self.mode {
                Mode::Get => client.get(&self.url),
                Mode::Post(ref json) => client.post(&self.url).json(json),
            };

            Ok(req.headers(headers).send().await?)
        }
    }
}
