use matchit::Router;
use mosquitto_rs::{Client, Message, QoS};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

pub type MqttHandlerResult = anyhow::Result<()>;

/// A helper struct to type-erase handler functions for the router
struct Dispatcher<S = ()>
where
    S: Clone,
{
    func: Box<dyn Fn(JsonValue, Message, S) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>>,
}

impl<S: Clone + 'static> Dispatcher<S> {
    pub async fn call(&self, params: JsonValue, message: Message, s: S) -> MqttHandlerResult {
        (self.func)(params, message, s).await
    }

    pub fn new<F, Fut, T>(func: F) -> Self
    where
        F: (Fn(T, Message, S) -> Fut) + Clone + 'static,
        Fut: Future<Output = MqttHandlerResult>,
        T: DeserializeOwned,
    {
        let wrap: Box<
            dyn Fn(JsonValue, Message, S) -> Pin<Box<dyn Future<Output = MqttHandlerResult>>>,
        > = Box::new(move |params: JsonValue, message: Message, state: S| {
            let func = func.clone();
            Box::pin(async move {
                let parsed: T = serde_json::from_value(params)?;
                func(parsed, message, state).await
            })
        });

        Self { func: wrap }
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
    pub async fn route<'a, P, T, F, Fut>(&mut self, path: P, handler: F) -> anyhow::Result<()>
    where
        P: Into<String>,
        F: (Fn(T, Message, S) -> Fut) + Clone + 'static,
        Fut: Future<Output = MqttHandlerResult>,
        T: DeserializeOwned,
    {
        let path = path.into();
        self.client
            .subscribe(&route_to_topic(&path), QoS::AtMostOnce)
            .await?;
        self.router.insert(path, Dispatcher::new(handler))?;
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
