# PowerView to MQTT bridge for Home Assistant

This repo provides the `pview` executable whose primary purpose is to act as a
bridge between a [Hunter Douglas PowerView
Hub](https://www.hunterdouglas.com/operating-systems/motorized/powerview) and
Home Assistant, via the [Home Assistant MQTT Integration](https://www.home-assistant.io/integrations/mqtt/).

`pview` also provides a few subcommands that can be used to interact with the
PowerView hub via the CLI.

## But Home Assistant already has a PowerView Integration!?

So why do this?

* [this polling issue that hangs the
  hub](https://github.com/home-assistant/core/issues/73900) has no one working
  to resolve it, and causes chronic recurring issues in my home.
* `pview` is able to use the `/api/homeautomation` webhook interface that is
  partially [documented in the REST
  API](https://github.com/openhab/openhab-addons/files/7583705/PowerView-Hub-REST-API-v2.pdf)
  to receive notifications from the hub, and allow the hub to intelligently
  refresh shades that fail post-move validation checks, eliminating
  the problematic need to manage force-refreshing of shade data.
* I'm much more comfortable and productive in a single Rust repo than working
  across the two separate Python projects that are required for home assistant
  development.

## How do I use this?

To run the mqtt bridge:

* Ensure that you have configured the MQTT integration:
  * [follow these steps](https://www.home-assistant.io/integrations/mqtt/#configuration)

* Prepare a `.env` file with your mqtt information:

```bash
# If you want to turn up debugging, uncomment the next line
#RUST_LOG=pview=debug
# Always colorize output when running in docker
RUST_LOG_STYLE=always
# The hostname or IP address of your mqtt broker
PV_MQTT_HOST=mqtt.localdomain
PV_MQTT_PORT=1883
# If you use authentication, uncomment and fill these out
#PV_MQTT_USER=username
#PV_MQTT_PASSWORD=password
```

* Set up your `docker-compose.yml`:

```yaml
version: '3.8'
services:
  pv2mqtt:
    image: ghcr.io/wez/pview:latest
    container_name: pv2mqtt
    restart: unless-stopped
    env_file:
      - .env
    # Host networking is required
    network_mode: host
```

* Launch it:

```console
$ docker compose up -d
```

* Your shades and scenes will now populate into home assistant

## Non-Docker

The docker image is currently only built for x86-64. If you run on another
architecture, or don't want to use docker, then you must build it yourself:

```console
$ cargo build --release
```

You can then install `target/release/pview` wherever you like.

### Running it

To start the bridge:

```console
$ pview serve-mqtt
```


## Limitations

* I can only directly test on the hardware that I have.
  There is a reasonable chance that if you have shades with
  tilt or other functionality that the behavior may not be optimal.
  Please file an issue and be prepared to do grab some diagnostics via `curl`.
