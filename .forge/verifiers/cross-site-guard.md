---
paths: [api/src/**]
severity: major
mode: warn
---
Does every Rocket route this diff adds or changes with a non-GET method (post, put, patch, delete) take a cross-site guard (`NotCrossSite`, `DevOnly`, or another guard that refuses cross-site requests) as a parameter, besides `LocalHost`?

The API has no login and binds 127.0.0.1, so any web page the user visits can send it requests. `LocalHost` (api/src/host.rs) checks only the `Host` header, which a cross-site request to 127.0.0.1 passes. A POST with a form, multipart or text body and no custom header is a CORS "simple request" that needs no preflight, and Rocket 0.5 rewrites a form POST whose first field is `_method=delete` (or put, patch) into that method, so no method is safe on its own.

`NotCrossSite` (api/src/google/mod.rs) refuses `Sec-Fetch-Site: cross-site` and any form or multipart body. `DevOnly` (api/src/dev.rs) requires the custom header `X-Waypoint-Dev: 1`, which forces a preflight the API never answers, plus a same-origin `Sec-Fetch-Site`. An upload route can't use `NotCrossSite`, so it needs a custom-header guard like `DevOnly`'s.

Answer NO only for a route attribute such as `#[rocket::post(...)]`, `#[post(...)]` or `#[rocket::delete(...)]` added or changed in this diff whose handler takes no such guard. GET routes, and routes the diff doesn't touch, are out of scope.
