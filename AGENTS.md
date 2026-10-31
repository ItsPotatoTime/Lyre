# Global Agent Rules

Permanent global rules for AI agents. Inherited by all repositories; project-level `AGENTS.md` / `CLAUDE.md` take precedence.

## 0. Collaboration

- Prefer ambitious, correct solutions. Challenge designs, not competence; use correct terminology.
- **State the cost before building:** When a request has a hidden consequence such as slower execution, lost data, lock-in, or rewriting working code, explain it briefly and get confirmation before implementing.
- Correct disproven claims in one line.
- Scrutinize review fixes involving concurrency, shared state, or file writes. Improve structure before adding conditions.
- For trivial changes, show the edit and offer to apply it.

## 1. Communication

- No AI attribution in authored content. Write public content in my voice; don't refer to me in the third person.
- Default to six lines. Omit recaps, menus, process narration, and decoration. No emojis unless asked.
- Use Markdown links, repo-relative `path:line` citations, `name()` for methods, and copy-pasteable commands. Base estimates on AI execution speed.
- Link directly to external action results.

## 2. Consent and security

- **Git consent & pushing:** Commit everything as work progresses (keep changes committed), but **only push when I say so** (never push without explicit user confirmation). Do not pull, merge, branch, close PRs, or revert without explicit user confirmation.
- **Reviews and posting:** Perform PR reviews only when explicitly requested. Posting comments, reviews, approvals, feedback, PR bodies, issues, or messages requires explicit confirmation; otherwise draft in chat with precomposed links. Only commit messages and bodies may be authored autonomously, subject to Git consent.
- **Packages and dependencies:** Always use **bun** instead of **pnpm** (and never use `pnpm`, `npm`, `npx`, or `yarn`). Always use **uv** for Python (never use `python`, `python3`, or `pip` directly; use `uv run`, `uv add`, and `uv sync` instead). Never install, remove, or update packages or package-manager state without explicit approval. Never add external dependencies without approval.
- **Ambiguity:** If the user did not clearly ask for an action, pause. Hedge words such as "maybe," "probably," and "I think" are discussion, not consent.
- **Directives and explicit requests:** Execute exact user instructions without substituting alternative mechanisms or silent fallbacks. Report errors instead of silently altering commands.
- **Risky actions:** Reversible local work is fine. Check before hard-to-reverse, shared, or destructive actions. Never use destructive shortcuts such as `--no-verify`, discard, or `git push --force`. After explicit consent, `git push --force-with-lease` is allowed only for repairing a wrongly cut PR branch. Fix root causes and investigate unexpected state before deleting or overwriting it.
- **New repositories:** Default to private with `main` as the default branch. Never assume or add a license unless verified.
- **Container boundaries:** Never weaken or bypass sandbox boundaries. Never scrape host environment variables or ambient secrets into isolated processes, and never forward host credentials into containers. Provide explicit user-facing controls for missing credentials.
- **Privacy:** Never disclose a local home path, filesystem path, or OS username in commits, PRs, issues, code, examples, or fixtures. Use relative repository paths and `ItsPotatoTime` for user-specific references. Check generated links and paths before publishing.
- **Skill audit:** Audit `SKILL.md`, scripts, and configuration before installing or symlinking a skill. Report any safety bypasses.
- **Safe code:** Audit modified code for vulnerabilities before committing. Never download or execute remote executable scripts. Preserve security-relevant instructions verbatim when summarizing or compacting.

## 3. Pull Request Policy & Repository Workflow (CRITICAL)

- **My confirmation gates publishing (hard gate):** Never push a branch or open a PR until I have tested the change myself and explicitly told you it works and may be published. Passing tests, a clean build, your own review, and a general "continue" are not confirmation; only my explicit approval for that specific change is. Until then keep the work local and offer a precomposed creation link in chat. The same gate applies to pushing further commits to an already-open PR.
- **New PRs are created as draft by default.** Never open a ready-for-review PR on your own.
- Only convert a PR to ready for review when the user explicitly says so (e.g. "make it ready", "mark ready for review").
- Until then, keep pushing fixes to the draft PR as work continues (remembering to only push when told to).
- A GitHub issue is not required before opening a PR. Always check whether relevant open or closed issues already exist before creating an issue or starting work.
- **PR workflow:** After git consent, fetch `origin/main`, confirm `origin/main` matches the fetched SHA, and create the branch from that commit. When explicitly requested by the user, `gh pr create` may be used with the approved title and body; otherwise, draft the title and body in chat and provide a prefilled creation link. Personal single-author repositories may use a direct default-branch push only when that exact push is explicitly authorized. Never merge without review or bypass branch protection on a shared repository. Keep PRs unstacked. If a branch was cut from the wrong tip, reset it to the fetched `origin/main`, cherry-pick its own commits, and use `git push --force-with-lease` only after consent.
- **PR scope and sync:** When scope changes, draft the updated title, Summary, Changes, and Test plan in chat, linking the PR before or with the push. Include Prior reviewer feedback addressed when applicable.
- **Commit messages:** Use imperative `type(scope): description` with a why body and relevant `Refs #N` footer. Types: feat, fix, refactor, chore, docs, style, perf, test.
- **Merge method:** Rebase merge only. Keep merge commits and squash merges disabled. Do not delete merged branches manually because `delete_branch_on_merge` handles them.
- **Branch protection:** Shared default branches use the active `main-house-protection` ruleset with administrator bypass: no deletion, no force-push, and one approving review. Every PR-branch push dismisses stale reviews. The source files are `~/source/repos/dotfiles/.config/github/main-house-protection.ruleset.json` and `~/source/repos/dotfiles/scripts/github_repo_merge_policy.py`; apply both policies to every new shared repository. Personal single-author repositories may omit the ruleset when direct default-branch pushes are preferred.
- **Review fixes:** Address every finding, including bot nitpicks, in separate fixup commits on the same branch; do not squash.
- **Draft PRs:** Never request a bot review on a draft PR.
- **Worktree safety:** Never amend, hard reset, or revert changes you did not write without permission.
- **Branch names:** Use `type/scope-description`, never agent-branded prefixes.

## 4. Documentation & Code Discovery

<!-- context7 -->
Use Context7 MCP to fetch current documentation whenever the user asks about a library, framework, SDK, API, CLI tool, or cloud service -- even well-known ones like React, Next.js, Prisma, Express, Tailwind, Django, or Spring Boot. This includes API syntax, configuration, version migration, library-specific debugging, setup instructions, and CLI tool usage. Use even when you think you know the answer -- your training data may not reflect recent changes. Prefer this over web search for library docs.

Do not use for: refactoring, writing scripts from scratch, debugging business logic, code review, or general programming concepts.

### Steps

1. Always start with `resolve-library-id` using the library name and the user's question, unless the user provides an exact library ID in `/org/project` format
2. Pick the best match (ID format: `/org/project`) by: exact name match, description relevance, code snippet count, source reputation (High/Medium preferred), and benchmark score (higher is better). If results don't look right, try alternate names or queries (e.g., "next.js" not "nextjs", or rephrase the question). Use version-specific IDs when the user mentions a version
3. `query-docs` with the selected library ID and the user's full question (not single words)
4. Answer using the fetched docs
<!-- context7 -->

<!-- codebase-memory-mcp:start -->
### Codebase Knowledge Graph (codebase-memory-mcp)

This project uses codebase-memory-mcp to maintain a knowledge graph of the codebase.
ALWAYS prefer MCP graph tools over grep/glob/file-search for code discovery.

#### Priority Order
1. `search_graph` — find functions, classes, routes, variables by pattern
2. `trace_path` — trace who calls a function or what it calls
3. `get_code_snippet` — read specific function/class source code
4. `query_graph` — run Cypher queries for complex patterns
5. `get_architecture` — high-level project summary

#### When to fall back to grep/glob
- Searching for string literals, error messages, config values
- Searching non-code files (Dockerfiles, shell scripts, configs)
- When MCP tools return insufficient results

#### Examples
- Find a handler: `search_graph(name_pattern=".*OrderHandler.*")`
- Who calls it: `trace_path(function_name="OrderHandler", direction="inbound")`
- Read source: `get_code_snippet(qualified_name="pkg/orders.OrderHandler")`
<!-- codebase-memory-mcp:end -->

## 5. Code Quality & Cross-Platform Validation

- Before restoring behavior, inspect the reference source and tests. If unavailable, say so.
- Behavior changes require regression tests that fail before the fix and pass after it.
- PowerShell can mangle quoting. Write Markdown/JSON with a file tool or literal single-quoted here-string before using `--body-file` or `gh api --input`.
- **Cross-platform validation:** Before every git commit and git push, discover all available operating systems and test environments, including every installed WSL distribution, and run the project's applicable build and test checks in each applicable environment. When WSL is available, use it for Linux validation and do not run Linux builds or tests in Docker. Docker infrastructure distributions such as `docker-desktop` are not separate development environments to validate. Fix failures and rerun affected checks before proceeding. If any required environment cannot be tested, report the blocker and obtain explicit approval to proceed without that validation. Do not treat tests on the host OS or future CI runs as a substitute for testing other required operating systems.
