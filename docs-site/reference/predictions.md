# How OIS predicts a flight

Every time OIS shows for a flight, at an FCA line, a fix, a sector or the runway, comes from one
prediction. It is remade from the latest VATSIM position each time a page asks, so an ETA moves when the
aircraft does. This page says what the prediction assumes, so you can tell when to trust it.

## What it starts from

- **Position, altitude, groundspeed and heading** from VATSIM.
- **The filed route**, from where the aircraft is now. If it can't be read, the straight line to the
  destination.
- **The filed altitude and speed.** An altitude it can't read is taken as FL350. A speed outside the type's
  range is brought into it.
- **Winds and temperatures aloft**, read at each point of the flight at the altitude the aircraft will be at there.
- **The aircraft type's performance**: ERAM's own climb and descent tables, unless a planner has entered a
  [profile for that type](/planning/aircraft-profiles).

## Climb, cruise and descent

A flight on the ground leaves at its release or EDCT, or 10 minutes after its filed departure time, and
climbs at its type's rate to its filed altitude. No taxi time is added.

In cruise it flies its filed true airspeed through the wind.

The descent is 1,000 feet for every 3 miles, down to the field's elevation: 30 miles out it is at 10,000
feet above the field. Descent speeds start about 10 miles before the type's own descent would. An aircraft
below that path stays level until the path comes down to it, and is never predicted climbing back up. One
above it is put on the path straight away.

On a trip too short to reach its filed altitude, the aircraft climbs until it meets the descent path and
comes down from there.

## An aircraft below its filed altitude

VATSIM gives no vertical speed, so OIS can't see whether an aircraft is climbing. It decides by distance. Once
an aircraft is well past where its type would have finished climbing, one and a half times that distance
from its departure, it is taken to be level where it is, until its descent. Closer in than that, it is taken
to be still climbing. An arrival stepped down early is the usual case: it stays in the sectors it is in.

## Groundspeed and assigned speeds

A level aircraft is predicted at the speed it is doing. OIS compares its groundspeed with what it
expected there, and speeds up or slows down the rest of the flight to match, by up to 15 percent. A climbing
aircraft isn't adjusted.

An aircraft in its descent that is more than 15 knots indicated slower than expected is taken to be on an
assigned speed. It is predicted no faster than that all the way in, so a delay you absorb with speed control
shows in its ETA. Take the restriction off and the ETA comes forward at the next update.

## What it can't see

- **Vectors, holds and shortcuts** that aren't in the flight plan. It flies the filed route from the aircraft's
  position, so a vector shows up as the aircraft moves.
- **Crossing restrictions.** STAR altitudes and speeds are not flown.
- **Slow or fast climbs.** Most airliners on VATSIM climb faster than ERAM's tables: the A320 reaches
  cruise in about 30 percent less distance. A departure's altitude on its way up reads low, and its first ETAs a
  little late.
- **Shallow descents.** Jets on VATSIM usually come down nearer 250 to 300 feet a mile than 333.
- **Taxi.** Release times are wheels-up times.
