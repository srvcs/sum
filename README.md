# srvcs-sum

The list-aggregation service of the srvcs.cloud distributed standard library.

Its single concern: **the sum of a list of integers.** It does no arithmetic of
its own. It folds the list through [`srvcs-add`](https://github.com/srvcs/add),
starting from `0`:

```text
acc = 0
for v in values:
    acc = add(acc, v)   # one HTTP call to srvcs-add per element
```

The sum of the **empty list** is `0`, and makes no dependency calls at all.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Sum the integers in `values` |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"values": [1, 2, 3, 4]}'
# {"values":[1,2,3,4],"result":10}
```

Responses:

- `200 {"values": [...], "result": n}` — evaluated.
- `422` — an element is not a valid integer, forwarded from `srvcs-add`.
- `500` — `srvcs-add` returned an unusable response.
- `503` — the `srvcs-add` dependency is unavailable.

## Dependencies

- [`srvcs-add`](https://github.com/srvcs/add)

A single request fans out across the dependency graph: one `sum → add` call per
list element, and each `add` in turn validates both operands via
`add → isnumber`.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_ADD_URL` | `http://127.0.0.1:8081` | Base URL of `srvcs-add` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-add` in-process that **actually
computes** `a + b` from the request body, so the fold is genuinely exercised
(e.g. `sum([1,2,3,4]) == 10`). See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
