use anyhow::anyhow;
use matchit::Router;
use mosquitto_rs::{Client, Message, QoS};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

pub type MqttHandlerResult = anyhow::Result<()>;

pub struct Request<S> {
    params: JsonValue,
    message: Message,
    state: S,
}

pub trait FromRequest<S>: Sized {
    fn from_request(request: &Request<S>) -> anyhow::Result<Self>;
}

pub struct Topic(pub String);
impl<S> FromRequest<S> for Topic {
    fn from_request(request: &Request<S>) -> anyhow::Result<Self> {
        Ok(Self(request.message.topic.clone()))
    }
}

pub struct Payload<T>(pub T);
impl<S, T> FromRequest<S> for Payload<T>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Debug,
{
    fn from_request(request: &Request<S>) -> anyhow::Result<Payload<T>> {
        let s = std::str::from_utf8(&request.message.payload)
            .map_err(|err| anyhow!("payload is not utf8: {err:#?}"))?;
        let result: T = s
            .parse()
            .map_err(|err| anyhow!("failed to parse payload {s}: {err:#?}"))?;
        Ok(Self(result))
    }
}

pub struct Params<T>(pub T);
impl<S, T> FromRequest<S> for Params<T>
where
    T: DeserializeOwned,
{
    fn from_request(request: &Request<S>) -> anyhow::Result<Params<T>> {
        let parsed: T = serde_json::from_value(request.params.clone())?;
        Ok(Self(parsed))
    }
}

pub struct State<S>(pub S);
impl<S> FromRequest<S> for State<S>
where
    S: Clone,
{
    fn from_request(request: &Request<S>) -> anyhow::Result<State<S>> {
        Ok(Self(request.state.clone()))
    }
}

/// A helper struct to type-erase handler functions for the router
pub struct Dispatcher<S = ()>
where
    S: Clone,
{
    func: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>>,
}

impl<S: Clone + 'static> Dispatcher<S> {
    pub async fn call(&self, params: JsonValue, message: Message, state: S) -> MqttHandlerResult {
        (self.func)(Request {
            params,
            message,
            state,
        })
        .await
    }

    pub fn new(
        func: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>>,
    ) -> Self {
        Self { func }
    }
}

pub trait MakeDispatcher<T, S: Clone> {
    fn make_dispatcher(func: Self) -> Dispatcher<S>;
}

impl<F, S, Fut> MakeDispatcher<(), S> for F
where
    F: (Fn() -> Fut) + Clone + 'static,
    Fut: Future<Output = MqttHandlerResult>,
    S: Clone + 'static,
{
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let wrap: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>> =
            Box::new(move |_request: Request<S>| {
                let func = func.clone();
                Box::pin(async move { func().await })
            });

        Dispatcher::new(wrap)
    }
}

impl<F, S, Fut, P1> MakeDispatcher<(P1,), S> for F
where
    F: (Fn(P1) -> Fut) + Clone + 'static,
    Fut: Future<Output = MqttHandlerResult>,
    S: Clone + 'static,
    P1: FromRequest<S>,
{
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let wrap: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>> =
            Box::new(move |request: Request<S>| {
                let func = func.clone();
                Box::pin(async move {
                    let p1 = P1::from_request(&request)?;
                    func(p1).await
                })
            });

        Dispatcher::new(wrap)
    }
}

impl<F, S, Fut, P1, P2> MakeDispatcher<(P1, P2), S> for F
where
    F: (Fn(P1, P2) -> Fut) + Clone + 'static,
    Fut: Future<Output = MqttHandlerResult>,
    S: Clone + 'static,
    P1: FromRequest<S>,
    P2: FromRequest<S>,
{
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let wrap: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>> =
            Box::new(move |request: Request<S>| {
                let func = func.clone();
                Box::pin(async move {
                    let p1 = P1::from_request(&request)?;
                    let p2 = P2::from_request(&request)?;
                    func(p1, p2).await
                })
            });

        Dispatcher::new(wrap)
    }
}

impl<F, S, Fut, P1, P2, P3> MakeDispatcher<(P1, P2, P3), S> for F
where
    F: (Fn(P1, P2, P3) -> Fut) + Clone + 'static,
    Fut: Future<Output = MqttHandlerResult>,
    S: Clone + 'static,
    P1: FromRequest<S>,
    P2: FromRequest<S>,
    P3: FromRequest<S>,
{
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let wrap: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>> =
            Box::new(move |request: Request<S>| {
                let func = func.clone();
                Box::pin(async move {
                    let p1 = P1::from_request(&request)?;
                    let p2 = P2::from_request(&request)?;
                    let p3 = P3::from_request(&request)?;
                    func(p1, p2, p3).await
                })
            });

        Dispatcher::new(wrap)
    }
}

impl<F, S, Fut, P1, P2, P3, P4> MakeDispatcher<(P1, P2, P3, P4), S> for F
where
    F: (Fn(P1, P2, P3, P4) -> Fut) + Clone + 'static,
    Fut: Future<Output = MqttHandlerResult>,
    S: Clone + 'static,
    P1: FromRequest<S>,
    P2: FromRequest<S>,
    P3: FromRequest<S>,
    P4: FromRequest<S>,
{
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let wrap: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>> =
            Box::new(move |request: Request<S>| {
                let func = func.clone();
                Box::pin(async move {
                    let p1 = P1::from_request(&request)?;
                    let p2 = P2::from_request(&request)?;
                    let p3 = P3::from_request(&request)?;
                    let p4 = P4::from_request(&request)?;
                    func(p1, p2, p3, p4).await
                })
            });

        Dispatcher::new(wrap)
    }
}

pub struct MqttRouter<S>
where
    S: Clone,
{
    router: Router<Dispatcher<S>>,
    client: Client,
}

impl<S: Clone + 'static> MqttRouter<S> {
    pub fn new(client: Client) -> Self {
        Self {
            router: Router::new(),
            client,
        }
    }

    /// Register a route from a path like `foo/:bar` to a handler function.
    /// The corresponding mqtt topic (`foo/+` in this case) will be subscribed to.
    /// When a message is received with that topic (say `foo/hello`) it will generate
    /// a parameter map like `{"bar": "hello"}`.
    /// That parameter map will then be deserialized into type `T` and passed as the
    /// first parameter of the handler function that is also passsed into `route`.
    pub async fn route<'a, P, T, F>(&mut self, path: P, handler: F) -> anyhow::Result<()>
    where
        P: Into<String>,
        F: MakeDispatcher<T, S>,
    {
        let path = path.into();
        self.client
            .subscribe(&route_to_topic(&path), QoS::AtMostOnce)
            .await?;
        let dispatcher = F::make_dispatcher(handler);
        self.router.insert(path, dispatcher)?;
        Ok(())
    }

    /// Dispatch an mqtt message to a registered handler
    pub async fn dispatch(&self, message: Message, state: S) -> anyhow::Result<()> {
        let topic = message.topic.to_string();
        let matched = self.router.at(&topic)?;

        let params = {
            let mut value_map = serde_json::Map::new();

            for (k, v) in matched.params.iter() {
                value_map.insert(k.into(), v.into());
            }

            if value_map.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::Object(value_map)
            }
        };

        matched.value.call(params, message, state).await
    }
}

/// A helper to deserialize from a string into any type that
/// implements FromStr
pub fn parse_deser<'de, D, T: FromStr>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    <T as FromStr>::Err: std::fmt::Display,
{
    use serde::de::Error;
    let s = String::deserialize(d)?;
    s.parse::<T>()
        .map_err(|err| D::Error::custom(format!("parsing {s}: {err:#}")))
}

/// Convert a Router route into the corresponding mqtt topic.
/// `:foo` is replaced by `+`.
fn route_to_topic(route: &str) -> String {
    let mut result = String::new();
    let mut in_param = false;
    for c in route.chars() {
        if c == ':' {
            in_param = true;
            result.push('+');
            continue;
        }
        if c == '/' {
            in_param = false;
        }
        if in_param {
            continue;
        }
        result.push(c)
    }
    result
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_route_to_topic() {
        for (route, expected_topic) in [
            ("hello/:there", "hello/+"),
            ("a/:b/foo", "a/+/foo"),
            ("hello", "hello"),
            ("who:", "who+"),
        ] {
            let topic = route_to_topic(route);
            assert_eq!(
                topic, expected_topic,
                "route={route}, expected={expected_topic} actual={topic}"
            );
        }
    }

    #[test]
    fn routing() -> anyhow::Result<()> {
        let mut router = Router::new();
        router.insert("pv2mqtt/home", "Welcome!")?;
        router.insert("pv2mqtt/users/:name/:id", "A User")?;

        let matched = router.at("pv2mqtt/users/foo/978")?;
        assert_eq!(matched.params.get("id"), Some("978"));
        assert_eq!(*matched.value, "A User");

        Ok(())
    }
}
