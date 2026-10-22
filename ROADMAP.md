# Waypoint roadmap

Work is organized into GitHub milestones: seven for the MVP (`v0.1.0`) and eight for `1.0.0`. Milestones in each release are listed in build order.

**Stack:** SvelteKit (Svelte 5, TypeScript, Tailwind) static SPA, a Rust API on Rocket, and PostgreSQL. Everything runs locally, and model calls go through `claude -p` on a Claude subscription instead of the API.

## How the MVP covers the agentic goals

| Goal | Where |
| --- | --- |
| Memory and a user model | MVP 4 builds the profile, preferences, answer bank and learning loop; MVP 5 and MVP 6 read from it and write signals back. |
| Multi-step planning | MVP 6's planner turns a vague goal into an editable plan of sourcing, scoring and prep steps, and replans when a step fails. |
| Calling messy external APIs | Gmail MIME and threads plus Google Calendar time zones (MVP 5), Greenhouse, Lever and Ashby with different schemas (MVP 6), and arbitrary job pages (MVP 2). These are Waypoint's equivalent of flights, hotels and maps; a maps API arrives with commute fit in 1.0.0. |
| Acting on the user's behalf | MVP 3 adds permission levels, approval gates, undo and the review queue; MVP 5 applies status updates with undo and creates calendar events only after confirmation; MVP 6 stops every application at an approval gate. |

## MVP

### MVP 1 · Foundations

A local app skeleton with the design system and plumbing every later milestone builds on.

**Scope**
- Stack ADR: SvelteKit (Svelte 5, TypeScript, Tailwind) built as a static SPA; Rust API on Rocket; PostgreSQL with sqlx migrations. Waypoint stands alone; nothing is shared with other projects
- Runs locally only: the API binds to `127.0.0.1`, Postgres runs in Docker Compose, and one command starts everything
- TypeScript types generated from the API's OpenAPI spec so the frontend and backend can't drift
- CI runs `cargo fmt`, `clippy`, `cargo test`, `svelte-check`, ESLint and Vitest
- Google OAuth with a localhost redirect to connect Gmail and Calendar; tokens encrypted at rest. Publish the OAuth app as "In production" (unverified, personal use), because "Testing" refresh tokens expire every 7 days
- Claude runner: a Rust module that spawns `claude -p` with `stream-json` input and output, using your subscription login. Never pass `--bare`, which ignores the login and requires an API key
- Claude runner isolation: `--tools ""`, `--strict-mcp-config`, `--no-session-persistence`, a clean working directory, and `--json-schema` for structured output
- Claude runner logs model, tokens, duration and reported cost for every call
- Postgres-backed job queue (`FOR UPDATE SKIP LOCKED`) with a concurrency limit and backoff when subscription usage limits are hit
- Server-sent events from the API so the UI updates live
- Design tokens: 4px spacing scale, color, type, radius, shadow, light and dark themes
- Motion tokens (durations, easings) and a `prefers-reduced-motion` fallback
- Base primitives: Button, IconButton, Link, Input, Textarea, Select, Checkbox, Card, Badge, Tabs, Dialog, Drawer, Toast, Tooltip, EmptyState, Skeleton
- Primitives own hover, focus-visible, active and disabled states, including `cursor-pointer` / `cursor-not-allowed`

**Done when:** One command starts the app locally, a test prompt round-trips through `claude -p` with no API key set, and the empty dashboard is built only from the primitives.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### MVP 2 · Tracker core

Replace the spreadsheet: track every application, its interview stages and the resume that went with it.

**Scope**
- Applications: company, title, URL, location, comp, source, notes
- Statuses: sourced, applied, interviewing, offer, rejected; board and list views with animated moves between columns
- A custom list of stages per application (add, reorder, complete) and a timeline of events
- Resume versions: upload PDF, keep parsed text, record which version went to which application
- Capture a job from a pasted URL: JSON-LD `JobPosting` first, Greenhouse/Lever/Ashby page detection next, LLM extraction as the fallback; user confirms the extracted fields
- Log rejection feedback against the application

**Done when:** Real applications are tracked here from week 1, and the spreadsheet is retired.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### MVP 3 · Agent runtime & guardrails

The shared machinery that lets agents act on your behalf safely: runs, tools, approval gates, undo and evals.

**Scope**
- Run model: every workflow run persists its steps, tool calls, inputs, outputs, status and cost, and can resume after a failure
- Tool registry with typed schemas and a permission level per tool: `read`, `write` (reversible) or `external` (leaves Waypoint)
- Agent tools exposed to `claude -p` as a local Waypoint MCP server via `--mcp-config`; no other tools are available to the agent
- Approval gate: `external` tools pause the run and create a pending action you approve, edit or reject
- Action log with one-click undo for every reversible write an agent makes
- Review queue UI shared by every workflow for low-confidence results and pending actions
- Run inspector UI: steps, tool calls, memories used, tokens and cost
- Eval harness: labelled datasets per workflow, a command that runs and scores them, tokens per run
- Kill switch: pause all agents; delete all data and revoke Google access

**Done when:** A sample workflow runs end to end, pauses at the approval gate, can be undone, and shows up in the run inspector.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### MVP 4 · Memory & user model

Waypoint learns your preferences and facts about you, shows you what it learned, and uses it in later runs.

**Scope**
- Candidate profile from LinkedIn's data export (positions, skills, education) plus parsed resumes
- Preferences: target roles, seniority, locations and remote, comp floor, industries, deal-breakers; stated directly or learned
- Learning loop: approvals, edits to drafts, dismissed suggestions and their reasons become signals; a memory writer proposes new facts or preferences with the evidence, and you confirm them
- Answer bank: canonical answers (work authorization, notice period, salary expectations, why you're leaving) that improve as you edit drafts
- Retrieval: workflows pull relevant memories into context and record which ones they used. Start with Postgres full-text search and tags; add pgvector with a local embedding model only if recall is poor
- "What Waypoint knows about me" page: view, edit and delete each memory, with where it came from

**Done when:** Correcting a draft once changes the next draft, and the run inspector shows which memory caused it.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### MVP 5 · Email-to-status pipeline

Recruiter emails keep application statuses, stages and interview times current without manual updates.

**Scope**
- Gmail sync with read-only scope, incremental via `historyId`, with recruiter and ATS sender heuristics. Cheap heuristics run before Claude so only likely recruiter emails use subscription usage
- Backfill and catch-up after the laptop was off run through the job queue, paced to stay inside usage limits
- Messy input handling: multipart MIME, HTML to text, quoted replies, threads, ATS no-reply senders (Greenhouse, Lever, Ashby, Workday)
- Classify each email (rejection, interview invite, availability request, offer, other) with a confidence score
- Extract company, role, stage name, interviewer and proposed times, with time zones normalized
- Match to an application by thread, sender domain, company and role; propose a new application when nothing matches
- Act: high-confidence status and stage updates apply with undo; everything else goes to the review queue
- Calendar: propose interview events and create them only after you confirm (`calendar.events` scope)
- Eval set: 50–100 of your own labelled recruiter emails; report classification accuracy, match accuracy and tokens per email

**Done when:** A week of your real inbox is processed, statuses match reality, and the eval numbers are recorded.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### MVP 6 · Goal planner & application prep

Turn a vague goal into a plan, find matching roles, and prepare applications that stop for your approval.

**Scope**
- Goal input, for example "senior frontend roles at climate companies in NYC, apply to 5 this week"
- Planner agent: asks clarifying questions, then produces an editable plan (target companies, sourcing, shortlist, prep); replans when a step fails or you change it
- Sourcing from target companies' public boards: Greenhouse, Lever and Ashby APIs normalized into one Job schema (different pagination, fields, HTML descriptions, location formats)
- Fit scoring against the user model with cited reasons
- Application prep: pick a resume version with reasons, draft answers from the answer bank and memory, assemble an application packet
- Approval gate: you approve or edit each packet; marking it applied records the resume version and answers used. Nothing is submitted automatically
- Your edits feed the memory learning loop

**Done when:** One real goal runs from vague request to approved packets that you submit yourself.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### MVP 7 · Dogfood & MVP release

Use Waypoint daily for two weeks, fix what gets in the way, and tag the MVP.

**Scope**
- Two weeks of daily use for the real job search; log friction as issues
- Fix the top friction issues and the review queue's most common false positives
- Usage per workflow and per day (runs, tokens, usage-limit hits) visible on the dashboard
- Accessibility pass: keyboard navigation, focus order, contrast
- Tag `v0.1.0`

**Done when:** Waypoint is the only place the job search is tracked, and `v0.1.0` is tagged.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

## 1.0.0

### 1.0.0 · Browser extension

Capture any job you're viewing in one click, including on LinkedIn, without automating LinkedIn.

**Scope**
- Extension that captures the current job page into Waypoint, reusing the URL extractor
- Talks to the local API; CORS allows only the extension's origin
- Duplicate detection against existing applications
- Add a captured job to an active goal
- Foundation for form filling (used by Assisted apply)

**Done when:** Jobs are captured from LinkedIn and company sites while browsing, with no duplicates.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Continuous sourcing

Watch target companies on a schedule and surface only roles worth your time.

**Scope**
- Scheduled crawling of target companies' Greenhouse, Lever and Ashby boards while the app runs, catching up on start
- Notifications for new roles that match
- Fit scoring with cited reasons, tuned with an eval set
- Ghost-job flags: posting age, repost frequency, unchanged listings
- Company briefs: what they do, size, funding, recent news
- Location and commute fit using a maps/geocoding API

**Done when:** New matching roles show up within a day of posting, with reasons you agree with.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Assisted apply

Waypoint fills application forms; you approve every submission.

**Scope**
- Tailored resume per job, shown as a diff against the base version
- Form filling through the extension from the application packet
- Full preview before submitting; every submission needs your click, and nothing is sent in bulk
- Record the exact packet that was sent

**Done when:** Applying to a prepared job takes one review and one click.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Interview prep agent

Prep for every stage, and learn from each round to prepare for the next.

**Scope**
- Per-stage prep: company research, likely questions, matching stories
- STAR story bank linked to the candidate profile
- Mock interviews
- Post-interview debrief that adjusts prep for the next round (long-running state across stages)

**Done when:** Every scheduled interview has a prep page, and every finished one has a debrief.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Follow-ups & growth

Never miss a follow-up, and turn rejections into a plan.

**Scope**
- Follow-up nudges when an application goes quiet
- Thank-you note drafts saved as Gmail drafts (compose scope only); you send them
- Rejection feedback turned into tracked goals
- Reapply reminders with a draft note

**Done when:** Follow-ups and thank-yous are drafted on time, and feedback has goals attached.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Insights

See what's working in the search.

**Scope**
- Funnel analytics: response rates by source, role type and resume version
- Which resume versions get responses
- Agent usage and accuracy trends per workflow

**Done when:** You can answer "which resume and which source get me interviews?" from the dashboard.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Offers & networking

Help at the end of the funnel and with warm introductions.

**Scope**
- Offer comparison across comp, equity, benefits and your stated preferences
- Negotiation prep
- Referral finder from LinkedIn's connections export
- Outreach drafts that you send yourself

**Done when:** Offers can be compared side by side, and every target company shows any known connections.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).

### 1.0.0 · Hardening & 1.0.0 release

Make Waypoint trustworthy enough to rely on, then tag 1.0.0.

**Scope**
- Export everything (applications, resumes, emails processed, memories) and delete everything
- Security review: OAuth scopes, token storage, data at rest, localhost-only binding
- Scheduled local backups of the Postgres database (`pg_dump`) with a tested restore
- Every workflow has an eval set and an eval command; evals run locally before merging, because they need your `claude` login
- Usage budgets per workflow so background jobs can't use up the subscription's limits
- Onboarding flow for a fresh account
- Accessibility and performance audit
- Tag `v1.0.0`

**Done when:** `v1.0.0` is tagged with every workflow passing its evals, backups restoring, and full export and delete working.

UI follows the conventions in CLAUDE.md (4px grid, cursor-pointer, reusable components first, transitions).
