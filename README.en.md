# GeoLedger

[中文](README.md) · [English](README.en.md)

GeoLedger is a version control and collaborative editing service for spatial features. Teams organize datasets into projects, edit GeoJSON features in individual workspaces, review differences, resolve conflicts, and publish shared revisions. Historical queries, commit reversal, member roles, and audit records form part of that workflow.

The repository contains the Rust server `geoledger-server`, the remote `gl` CLI, a web console, and SDKs for Rust, Go, TypeScript / Node.js, and Python. The server manages data in SQLite + SpatiaLite or PostGIS. HTTP and gRPC requests use the same application engine.

The current source version is `0.3.0-alpha.4`, with storage format 16, defined in [Cargo.toml](Cargo.toml) and the [engine](crates/engine/src/lib.rs), respectively.

## Key Capabilities

| Capability | Behavior |
|---|---|
| Datasets and coordinate systems | Organize point, line, and polygon features with two- or three-dimensional coordinates and an EPSG identifier or custom CRS. The default CRS is `EPSG:4326`. Saving and publishing preserve source coordinates and GeoJSON numeric values. |
| Workspaces and review | Edit multiple datasets in one workspace, save additions, updates, and deletions in batches, and inspect feature differences and dataset change summaries. Draft versions check concurrent edits. |
| Three-way merge and publication | Compare the base revision, current shared revision, and draft; merge independent property changes and review conflicting fields. Publication writes features, history, the revision, receipt, and audit records in one transaction. |
| Publication recovery | SDKs retain the publication request ID and original payload. Retrying the original request confirms the result after a connection interruption. The console provides a publication recovery flow. |
| Queries and commit reversal | Query by revision, feature ID, spatial bounds, and pagination cursor. Generate a workspace containing the inverse changes of a selected commit, then review and publish a new revision. |
| PostGIS table registration | Connect an existing business table to a dataset and create an initial version. Publication updates the source table transactionally and retains its CRS. |
| GIS console | Map, layer tree, feature search, attribute table, GeoJSON editing and upload, workspace version graph, conflict review, and mobile layouts. Map previews transform coordinates to WGS 84 as needed. |
| Identity and access | User management, project member roles, personal API keys, database sessions, and HTTP/gRPC TLS and mTLS configuration. |

## Get the Code

```sh
git clone https://github.com/mapseekai/geoledger.git
cd geoledger
```

Run the shell commands below from the repository root.

## Requirements

| Task | Tools and dependencies |
|---|---|
| Rust builds | Rust 1.88+, Cargo, protoc, a C toolchain, and CMake; Linux/macOS also require pkg-config and OpenSSL development libraries. Regular Rust CI checks use 1.92.0. |
| SQLite service | The native SpatiaLite extension and its dependencies. Set `GL_SPATIALITE_EXTENSION` to select its path. The repository Dockerfile includes the runtime. |
| PostGIS service | A PostgreSQL database with the PostGIS extension enabled and a service account. |
| Web and TypeScript SDK | Node.js 22 and npm. |
| Go SDK | Go 1.27.2+, as declared in [go.mod](sdk/go/go.mod). |
| Python SDK | Python 3.10+, as declared in [pyproject.toml](sdk/python/pyproject.toml). |
| Repository checks | Python 3.11+, Bash, rustfmt, and Clippy. Browser verification uses Python Playwright and Chromium/Chrome with a working sandbox. |
| Containers | Docker, Docker Compose, and OpenSSL to generate the session secret. |

See [Getting Started](docs/getting-started.md#环境准备) for Linux, macOS, and Windows setup. Rust dependencies use `Cargo.lock`; the web app and TypeScript SDK each have a `package-lock.json`. Install with `cargo --locked` and `npm ci`, respectively.

## Configuration

Server options are documented in `.env.example` and `geoledger-server --help`; console options are in `web/.env.example`. Load the server variables you need into the process environment. Compose interpolates variables from `.env`, and Next.js reads `web/.env.local`.

| Variable | Default or setup | Purpose |
|---|---|---|
| `GL_STORAGE` | `sqlite` | Select `sqlite` or `postgis`. |
| `GL_DATA_DIR` | `./geoledger-data` | Server data directory. |
| `GL_HTTP_LISTEN` | `127.0.0.1:7881` | HTTP listener. |
| `GL_GRPC_LISTEN` | `127.0.0.1:7882` | gRPC listener. |
| `GL_DATABASE_URL` | Set for PostGIS deployments | Database connection and TLS parameters. |
| `GL_SPATIALITE_EXTENSION` | Set to the installed path | SpatiaLite extension file. |
| `GL_ADMIN_USERNAME` | `admin` | Initial administrator name. |
| `GL_ADMIN_PASSWORD` | Optional before initialization | Initial password; when omitted, a random password is generated and written to a private file. |
| `GL_WEB_ENDPOINT` | `http://127.0.0.1:7881` | Private HTTP service used by the console. |
| `GL_WEB_ORIGIN` | `http://localhost:3000` | Exact origin used by the browser. |
| `GL_WEB_SESSION_SECRET` | Random value of at least 32 characters | Encrypt console session cookies. |
| `GL_ENDPOINT` | `http://127.0.0.1:7882` | gRPC service used by `gl`. |

Select a CLI credential source: `GL_API_KEY` or a private file through `GL_API_KEY_FILE` / `--api-key-file`. See [Security Configuration](docs/security.md) and [Production Operations](docs/production.md) for certificates, private CAs, request budgets, rate limits, and shutdown settings.

## Quickstart

### Run with Containers

```sh
export GL_WEB_SESSION_SECRET="$(openssl rand -base64 48)"
docker compose up --build -d
curl --fail http://127.0.0.1:7881/ready
```

[compose.yaml](compose.yaml) builds and starts an image containing the server, CLI, and console. Data is stored at `/data` in the `geoledger-data` named volume. A successful readiness check returns `{"ok":true}`.

| Interface | Local address |
|---|---|
| Web console | `http://localhost:3000` |
| HTTP API | `http://127.0.0.1:7881` |
| gRPC | `http://127.0.0.1:7882` |

On first startup, generated administrator credentials are saved in `/data/initial-admin-<id>.json`. Read this initialization file in your own terminal:

```sh
docker compose exec geoledger sh -c 'cat /data/initial-admin-*.json'
```

Sign in to the console with that username and password, then complete the initial password change. Create a personal key on the API Key page and save it in your own private credential file for CLI or SDK use.

### Run the Service and Console Locally

Prepare SpatiaLite, build the binaries, and start the server in a dedicated terminal:

```sh
cargo build --locked --bins
./target/debug/geoledger-server --data-dir ./geoledger-data
```

The server creates the SQLite database and initial administrator, and logs the path to the private initialization credential file. Keep it running and prepare the console in another terminal:

```sh
npm ci --prefix sdk/ts
npm --prefix sdk/ts run build
npm ci --prefix web
cp web/.env.example web/.env.local
openssl rand -base64 48
```

Copy the generated random value into `GL_WEB_SESSION_SECRET` in `web/.env.local`. Set the service address and browser origin as shown above, then run:

```sh
npm --prefix web run dev
```

Open `http://localhost:3000`, sign in with the initial administrator credentials, change the password, and create an API key. Save the key as the plain-text file `./api-key`. On Unix, set private permissions and verify the CLI:

```sh
chmod 600 ./api-key
curl --fail http://127.0.0.1:7881/ready
./target/debug/gl --api-key-file ./api-key info
./target/debug/gl --api-key-file ./api-key projects
```

`info` reports the protocol, storage backend, and request specifications. See [Getting Started](docs/getting-started.md) for setup details and the [Console Guide](docs/console.md) for interface walkthroughs.

## Collaboration Workflow

1. **Organize data.** Create a project and datasets, choosing geometry type, coordinate dimension, and CRS. Stable string IDs identify features.
2. **Edit drafts.** Create a workspace and save complete GeoJSON Features or explicit deletion operations. Draft queries combine the base revision with workspace changes.
3. **Review changes.** Inspect `diff` and change summaries. As shared revisions advance, compare `base`, `current`, and `draft`, submit conflict resolutions, and update the workspace base.
4. **Publish a revision.** The request carries the workspace version and request ID. A successful transaction returns a receipt. Retries retain the same request ID and payload.
5. **Read history.** Query data and commit details at a revision. Paginated shared-data queries retain the first page's revision; spatial bounds use the dataset CRS.
6. **Reverse a commit.** `restore` creates a workspace with the inverse changes of a selected commit. Review it and publish a new shared revision.

See the [User Guide](docs/user-guide.md) for the data model and merge rules, and [API and SDKs](docs/api.md) for pagination and retry contracts.

`diff`, `commit`, `history`, and `audit` use server page cursors to mark the end of the current read snapshot. `history` returns a page containing `commits`; see the [pagination contract](docs/api.md#差异提交历史与审计分页).

## API and SDKs

HTTP business endpoints live under `/api/v1`. The four SDKs and CLI use the `geoledger.v1` gRPC protocol. SDKs authenticate with user API keys; workspace objects maintain draft versions and pending publication requests.

| Interface | Source and API |
|---|---|
| Rust | [sdk/rust](sdk/rust), asynchronous business client. |
| Go | [sdk/go](sdk/go), business calls with `context.Context`. |
| TypeScript / Node.js | [sdk/ts](sdk/ts), Promise APIs and exact JSON handling. |
| Python | [sdk/python](sdk/python), business objects and context managers. |
| CLI | [crates/cli](crates/cli), `info`, `projects`, `members`, `history`, and general-purpose `call`. |
| gRPC protocol | [geoledger.proto](proto/geoledger/v1/geoledger.proto). |

### Python Example

Install the SDK from the repository and use the `./api-key` file created above:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./sdk/python
```

This example creates a dataset, saves a monitoring station, publishes it, and reads the resulting revision:

```python
from pathlib import Path

from geoledger import Client

api_key = Path("api-key").read_text().strip()
with Client("http://127.0.0.1:7882", api_key) as client:
    project = client.create_project("Monitoring")
    dataset = client.create_dataset(
        project.id, "stations", "point",
        coordinate_dimension=2, crs="EPSG:4326",
    )
    workspace = client.create_workspace(project.id)
    workspace.save(dataset.id, {
        "type": "Feature",
        "id": "station-001",
        "properties": {"name": "Station 1", "sequence": 9007199254740993},
        "geometry": {"type": "Point", "coordinates": [120.15, 30.27]},
    })
    print(workspace.diff())
    receipt = workspace.publish("Add station-001")
    snapshot = client.features(
        project.id, dataset.id, revision=receipt["revision"],
    )
    print(receipt["revision"], snapshot)
```

The result contains the published revision and feature collection, with the integer value of `sequence` preserved. See [API and SDKs](docs/api.md) for installation by language, authentication, business errors, and publication recovery examples.

## Architecture

```text
Browser -> Next.js BFF -> private Session HTTP -> engine Application
HTTP API / gRPC -> server adapters -----------> engine Application
CLI / SDKs -> gRPC ---------------------------> engine Application
                                                      |
                                     StorageBackend / RepositoryTransaction
                                                      |
                                      SQLite + SpatiaLite / PostGIS
```

Browser sessions use random Session IDs inside encrypted HttpOnly cookies, with identity and expiry managed in the database. The application engine performs authorization, draft editing, and publication through storage transaction interfaces.

| Path | Responsibility |
|---|---|
| `crates/core` | Data models and three-way merge. |
| `crates/engine` | Identity, authorization, workspaces, history, publication, and storage adapters. |
| `crates/spatialite` | Native SpatiaLite connections and geometry runtime. |
| `crates/server`, `crates/rpc` | HTTP/gRPC, TLS, service configuration, and data operations. |
| `crates/cli`, `sdk` | Remote CLI and four language clients. |
| `web` | Next.js console, browser endpoints, and map workspace. |
| `deploy`, `scripts` | Deployment templates, verification scripts, and release gates. |

SQLite uses WAL, SpatiaLite geometry, and RTree spatial indexes. PostGIS uses `geometry` columns and GiST indexes. See [Storage Extensions](docs/storage.md) for transaction boundaries and backend conformance requirements.

## Build and Verification

```sh
cargo build --release --locked --bins
./scripts/check.sh
```

Binaries are written to `target/release/geoledger-server` and `target/release/gl`, with an `.exe` suffix on Windows. `check.sh` requires Python 3 and ast-grep 0.51.0 (`npm install --global @ast-grep/cli@0.51.0`). It validates the three-language test layout, then runs documentation and configuration checks, rustfmt, Clippy, engine and workspace tests, and a backup/restore drill. Set `GL_TEST_DATABASE_URL` to enable PostGIS cases against an isolated `geoledger_test` database.

Web checks and production build:

```sh
npm --prefix web test
npm --prefix web run lint
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

The [Development Guide](docs/development.md) provides commands for four-language SDK integration and real browser verification. Both console CI browser suites share sandbox configuration, and the Ubuntu 24.04 runner uses system Chrome. The dependency gate `./scripts/audit.sh` checks Rust, npm, Go, and Python advisories.

Measured results for this workspace fix, its execution environment, and outstanding verification items are recorded separately in the [2026-10-15 Validation Record](docs/ci-validation-2026-10-15.md).

## Deployment and Operations

The [Dockerfile](Dockerfile) packages the business service, CLI, Next.js standalone console, and SpatiaLite runtime into one image, running as UID 10001. Compose persists `/data`; the container entry point manages service lifecycles and health checks.

For PostGIS, configure `GL_STORAGE=postgis`, `GL_DATABASE_URL`, and the service account. Connections verify certificates and hostnames by default and accept private CA configuration. Existing business tables are connected through the SDK methods `register_table` / `registerTable` / `RegisterTable`. See [Production Operations](docs/production.md) for deployment templates, systemd, and HTTPS gateways.

| Operational interface | Behavior |
|---|---|
| `/health` | Reports process health and the service version. |
| `/ready` | Checks service admission state, available execution capacity, and storage structure. |
| `/metrics` | Exposes Prometheus metrics to authenticated users. |
| JSON logs | `RUST_LOG` controls the log level for requests and runtime diagnostics. |

Data operations include online SQLite backup and restore, plus logical export, import, and verification. These commands use new output files:

```sh
./target/release/geoledger-server --data-dir ./geoledger-data \
  backup --output ./geoledger-backup.sqlite3

./target/release/geoledger-server --data-dir ./geoledger-data \
  export --output ./geoledger-export.jsonl

./target/release/geoledger-server \
  verify --input ./geoledger-export.jsonl --file-only
```

Restore into a new data directory. Logical import writes to a new or empty database and validates file structure, per-table digests, and integrity checksums. See [Production Operations](docs/production.md) for procedures and identity-data preservation.

## Contributing

Use [GitHub Issues](https://github.com/mapseekai/geoledger/issues) to report the version, reproduction steps, and expected result. Read [AGENTS.md](AGENTS.md) before changing the repository, run the relevant checks from the [Development Guide](docs/development.md), and describe the validation actually performed.

Report vulnerabilities through the private channels in [SECURITY.md](SECURITY.md). Version changes and compatibility details are documented in [CHANGELOG.md](CHANGELOG.md) and the [Current Version Contract](docs/api.md#当前版本契约).

## Documentation

| Document | Contents |
|---|---|
| [Getting Started](docs/getting-started.md) | Platform setup, service startup, and client connections. |
| [User Guide](docs/user-guide.md) | Data model, collaboration, merge, history, and commit reversal. |
| [Console Guide](docs/console.md) | Map workspace, resource management, and user operations. |
| [API and SDKs](docs/api.md) | Interfaces, examples in four languages, pagination, and retries. |
| [Storage Extensions](docs/storage.md) | Backend interfaces, transaction semantics, and conformance tests. |
| [Production Operations](docs/production.md) | Deployment, database connections, and backup/restore. |
| [Security Configuration](docs/security.md) | TLS, passwords, API keys, and sessions. |
| [Development Guide](docs/development.md) | Builds, code generation, and tests. |
| [Release Acceptance](docs/release.md) | Release gates, recovery drills, and load verification. |

## License

GeoLedger is licensed under the [MIT License](LICENSE). See [Web Third-Party Notices](web/THIRD_PARTY_NOTICES.md) for console component, font, and other third-party licenses.
