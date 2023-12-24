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

```console
$ pview --help
Usage: pview <COMMAND>

Commands:
  list-scenes
  list-shades
  inspect-shade
  move-shade
  activate-scene
  serve-mqtt
  hub-info
  help            Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

#### Listing Shades

```console
$ pview list-shades --help
Usage: pview list-shades [OPTIONS]

Options:
      --room <ROOM>  Only return shades in the specified room
  -h, --help         Print help
```

```console
$ pview list-shades
ROOM             SHADE                               POSITION
Zen Room         Zen Room Middle                           0%
Zen Room         Zen Room Middle Middle Rail              26%
Zen Room         Zen Room Right                            0%
Zen Room         Zen Room Right Middle Rail               25%
Bedroom 1        BedRoom 1 Left                            0%
Bedroom 1        BedRoom 1 Left Middle Rail               25%
Bedroom 1        Bedroom 1 Right                           0%
Bedroom 1        Bedroom 1 Right Middle Rail              25%
```

#### Moving a Shade

```console
$ pview move-shade --help
Usage: pview move-shade <--motion <MOTION>|--percent <PERCENT>> <NAME>

Arguments:
  <NAME>  The name or id of the shade to open. Names will be compared ignoring case

Options:
      --motion <MOTION>    [possible values: down, heart, jog, left-tilt, right-tilt, stop, up, calibrate]
      --percent <PERCENT>
  -h, --help               Print help
```

```console
$ pview move-shade --motion up "bedroom 1 left"
```

#### Inspecting a Shade

```console
$ pview inspect-shade --help
Usage: pview inspect-shade <NAME>

Arguments:
  <NAME>  The name or id of the shade to inspect. Names will be compared ignoring case

Options:
  -h, --help  Print help
```

```console
$ pview inspect-shade "Zen room middle"
Primary(
    ShadeData {
        battery_status: High,
        battery_strength: 181,
        firmware: Some(
            ShadeFirmware {
                build: 3147,
                index: Some(
                    3,
                ),
                revision: 2,
                sub_revision: 3,
            },
        ),
        capabilities: TopDownBottomUp,
        battery_kind: HardWiredPowerSupply,
        smart_power_supply: SmartPowerSupply {
            status: 0,
            id: 0,
            port: 0,
        },
        signal_strength: None,
        motor: None,
        group_id: 37321,
        id: 59066,
        name: Some(
            Base64Name(
                "Zen Room Middle",
            ),
        ),
        order: None,
        positions: Some(
            ShadePosition {
                pos_kind_1: PrimaryRail,
                pos_kind_2: Some(
                    SecondaryRail,
                ),
                position_1: 0,
                position_2: Some(
                    17040,
                ),
            },
        ),
        room_id: Some(
            60490,
        ),
        secondary_name: None,
        shade_type: DuetteTopDownBottomUp,
    },
)
```

#### Working with Scenes

```console
$ pview list-scenes --help
Usage: pview list-scenes [OPTIONS]

Options:
      --room <ROOM>  Only return shades in the specified room
  -h, --help         Print help
```

```
$ pview list-scenes
SCENE/SHADES                     POSITION
Open Guest
    BedRoom 1 Left                 0% 25%
    Bedroom 1 Right                0% 25%
    Zen Room Middle                0% 25%
    Zen Room Right                 0% 25%
```

```console
$ pview activate-scene --help
Usage: pview activate-scene <NAME>

Arguments:
  <NAME>  The name or id of the shade to inspect. Names will be compared ignoring case

Options:
  -h, --help  Print help
```

#### Getting Hub Information

```console
$ pview hub-info --help
Usage: pview hub-info

Options:
  -h, --help  Print help
```

#### Running the MQTT Bridge

```console
$ pview serve-mqtt --help
Usage: pview serve-mqtt [OPTIONS]

Options:
      --host <HOST>
          The mqtt broker hostname or address. You may also set this via the PV_MQTT_HOST environment variable
      --port <PORT>
          The mqtt broker port You may also set this via the PV_MQTT_PORT environment variable. If unspecified, uses 1883
      --username <USERNAME>
          The username to authenticate against the broker You may also set this via the PV_MQTT_USER environment variable
      --password <PASSWORD>
          The password to authenticate against the broker You may also set this via the PV_MQTT_PASSWORD environment variable
      --bind-address <BIND_ADDRESS>

      --discovery-prefix <DISCOVERY_PREFIX>
          [default: homeassistant]
  -h, --help
          Print help
```

Recommendation is to put the mqtt options into a `.env` file and launch without parameters:

```console
$ pview serve-mqtt
```


## Limitations

* I can only directly test on the hardware that I have.
  There is a reasonable chance that if you have shades with
  tilt or other functionality that the behavior may not be optimal.
  Please file an issue and be prepared to do grab some diagnostics via `curl`.
