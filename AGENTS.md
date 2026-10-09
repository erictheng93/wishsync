# Repository Guidelines

## Project Structure & Module Organization

WishSync is a wishlist-sharing application with guest item claims.
- `apps/web/app/`: Nuxt 4 frontend; route pages, creator/guest components, composables, middleware, utilities, and CSS assets.
- `apps/web/public/`: static assets; `apps/web/server/`: server routes; `apps/web/tests/`: frontend logic tests.
- `apps/api/src/`: Rust 2021 API using axum and sqlx, organized by features such as authentication, wishlists, and claims. Integration tests live in `apps/api/tests/`.
- `db/migrations/`: numbered PostgreSQL/PostGIS migrations, applied automatically at API startup.
- `docs/`: product requirements, user flows, wireframes, and schema/API contracts; `design/`: theme reference.

## Build, Test, and Development Commands

Run these from the repository root unless indicated otherwise:
- `docker compose up -d`: start PostGIS, MinIO, and Mailpit.
- Export `DATABASE_URL=postgres://wishsync:wishsync@localhost:5432/wishsync` and `APP_ENV=dev` before starting the API.
- `cargo run --manifest-path apps/api/Cargo.toml`: serve the API on port 8080.
- `cargo build --manifest-path apps/api/Cargo.toml`: compile the backend.
- `cargo test --manifest-path apps/api/Cargo.toml`: run backend tests; `scripts/cargo.sh test` is the Docker alternative.
- In `apps/web`, run `npm ci`, then `npm run dev` for port 3000.
- In `apps/web`, run `npm run typecheck && npm test && npm run build` to reproduce frontend CI checks. CI uses Node.js 22.

## Coding Style & Naming Conventions

Match adjacent code: four-space Rust indentation and two-space TypeScript/Vue indentation. Use Rust `snake_case`, PascalCase Vue component filenames, and `useX` composable names. Preserve Nuxt file-based routing, including `[slug].vue`. No dedicated lint or formatter script is configured; keep formatting changes focused.

## Testing Guidelines

Frontend tests use Vitest and `tests/**/*.test.ts`. Backend database tests use `#[sqlx::test]` with isolated databases; provide a running PostGIS instance and a database role able to create databases. Add regression tests for changed behavior, especially claim concurrency, authorization, and rate limits. No numeric coverage threshold is configured.

## Commit & Pull Request Guidelines

Recent commits use `feat:` and `fix:` prefixes with concise Chinese descriptions. Follow that style. PRs should describe the behavior change, link relevant issues, record checks run, and include screenshots for UI changes. Flag migrations and configuration changes explicitly.

## Configuration & Agent Instructions

Use `apps/api/.env.example` and README configuration guidance. Keep secrets out of commits; `.env` is ignored. Production requires explicit security settings; use `APP_ENV=dev` locally.

For structural codebase exploration, use the installed `codebase-memory` skill.
