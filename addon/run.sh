#!/usr/bin/with-contenv bashio

export RUST_BACKTRACE=full
export RUST_LOG_STYLE=always

if bashio::config.has_value mqtt_host ; then
  export PV_MQTT_HOST="$(bashio::config mqtt_host)"
else
  export PV_MQTT_HOST="$(bashio::services mqtt 'host')"
fi

if bashio::config.has_value mqtt_port ; then
  export PV_MQTT_PORT="$(bashio::config mqtt_port)"
else
  export PV_MQTT_PORT="$(bashio::services mqtt 'port')"
fi

if bashio::config.has_value mqtt_username ; then
  export PV_MQTT_USER="$(bashio::config mqtt_username)"
else
  export PV_MQTT_USER="$(bashio::services mqtt 'username')"
fi

if bashio::config.has_value mqtt_pass ; then
  export PV_MQTT_PASSWORD="$(bashio::config mqtt_password)"
else
  export PV_MQTT_PASSWORD="$(bashio::services mqtt 'password')"
fi

env | grep PV_

exec /pview serve-mqtt
