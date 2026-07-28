---
name: libspec-agent-workflow
description: >-
  Executes, prototypes, and manages agent workflows using `uv run libspec agent-workflow`.
  Use this skill whenever prototyping agent workflows, testing libspec steps, or when asked to prototype using libspec.
---

# Libspec Agent Workflow Skill

## Overview
This skill standardizes instructions for prototyping, running, and debugging agent workflows within the `libspec` repository.

## Execution Rules
- Always run agent workflow prototyping commands via `uv run`:
  ```bash
  uv run libspec agent-workflow [subcommand / options]
  ```
- Use standard `uv` commands (`uv run`, `uv sync`) without invoking raw `python` interpreters directly.
- Inspect logs, file outputs, or CLI output produced by the workflow runs to verify results.
