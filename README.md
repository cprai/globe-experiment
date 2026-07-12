# Solar System

An astronomically-accurate, interactive 3D solar-system simulation with
satellite tracking, written in Rust. Sol, Luna, and the planets are placed
from the JPL DE440 ephemeris, Terra's orientation comes from measured
Earth-orientation data, body rotations follow the IAU models (Luna with true
libration), and satellites fly propagated orbits. Terra is rendered
physically lit - atmosphere, city lights, terrain relief, ocean glint -
Terra and Luna eclipse each other, and a star-fixed camera orbits any of the
nine bodies at true scale and distance. Scenes replay **past** events only.

## Building and running

```sh
cargo run --release -- scene solar_system    # bare `scene` lists all scenes
```

The first build needs a network connection: the textures (NASA-derived maps
and a star field from
[Solar System Scope](https://www.solarsystemscope.com/textures/)), the
planetary ephemeris, and the Earth-orientation data are downloaded once and
embedded, making the binary self-contained. Building requires a C compiler
and Python 3 with its development library.
