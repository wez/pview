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

macro_rules! impl_make_dispatcher {
    (
        [$($ty:ident),*], $last:ident
    ) => {

impl<F, S, Fut, $($ty,)* $last> MakeDispatcher<($($ty,)* $last,), S> for F
where
    F: (Fn($($ty,)* $last) -> Fut) + Clone + 'static,
    Fut: Future<Output = MqttHandlerResult>,
    S: Clone + 'static,
    $( $ty: FromRequest<S>, )*
    $last: FromRequest<S>
{
    #[allow(non_snake_case)]
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let wrap: Box<dyn Fn(Request<S>) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>> =
            Box::new(move |request: Request<S>| {
                let func = func.clone();
                Box::pin(async move {
                    $(
                    let $ty = $ty::from_request(&request)?;
                    )*

                    let $last = $last::from_request(&request)?;

                    func($($ty,)* $last).await
                })
            });

        Dispatcher::new(wrap)
    }
}

    }
}

#[rustfmt::skip]
macro_rules! all_the_tuples {
    ($name:ident) => {
        $name!([], T1);
        $name!([T1], T2);
        $name!([T1, T2], T3);
        $name!([T1, T2, T3], T4);
        $name!([T1, T2, T3, T4], T5);
        $name!([T1, T2, T3, T4, T5], T6);
        $name!([T1, T2, T3, T4, T5, T6], T7);
        $name!([T1, T2, T3, T4, T5, T6, T7], T8);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8], T9);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9], T10);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10], T11);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11], T12);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12], T13);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13], T14);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14], T15);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15], T16);
    };
}

all_the_tuples!(impl_make_dispatcher);

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
