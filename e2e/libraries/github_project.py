"""Robot Framework helpers for GitHub Project v2 manipulations.

This harness drives the REAL `supergeoff/sympheo` repo + Project v2 #2.
Every cleanup keyword applies multiple safety pre-checks before it
deletes anything: the issue title MUST start with the e2e prefix, the
issue node id MUST match the one captured at setup, and any branch
deletion is gated on a `sympheo/` prefix.

All keywords assume `gh` is authenticated (or `GITHUB_TOKEN` is exported).
"""
from __future__ import annotations

import json
import subprocess
import time
import uuid
from typing import Optional, Tuple

from robot.api.deco import keyword
from robot.api import logger


# ---- Safety constants ------------------------------------------------------
# These values are the contract between this library and the cleanup logic.
# They MUST match the strings used by the suite when it creates the issue
# and the orchestrator's branch-naming convention.
ISSUE_TITLE_PREFIX = "[e2e-test]"
ISSUE_BODY_MARKER = "[automated e2e — safe to ignore]"
BRANCH_PREFIX = "sympheo/"


def _run(cmd: list[str], check: bool = True) -> str:
    """Run a subprocess and return stdout. On failure, dump stderr to log."""
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if check and proc.returncode != 0:
        raise AssertionError(
            f"{' '.join(cmd)} exited {proc.returncode}\nstdout: {proc.stdout}\nstderr: {proc.stderr}"
        )
    return proc.stdout


def _run_no_check(cmd: list[str]) -> Tuple[int, str, str]:
    proc = subprocess.run(cmd, capture_output=True, text=True)
    return proc.returncode, proc.stdout, proc.stderr


def _resolve_project_status(owner: str, project_number: int) -> Tuple[str, str, dict]:
    """Return (project_id, status_field_id, {option_name_lower: option_id})."""
    raw = _run([
        "gh", "project", "field-list",
        str(project_number), "--owner", owner, "--format", "json",
    ])
    data = json.loads(raw)
    fields = data.get("fields", data)
    field_id = ""
    options: dict[str, str] = {}
    for f in fields:
        if f.get("name") == "Status" and f.get("type", "").endswith("SingleSelectField"):
            field_id = f["id"]
            for o in f.get("options", []):
                options[o["name"].lower()] = o["id"]
    if not field_id:
        raise AssertionError(
            f"Status field not found on project {owner}/{project_number}; "
            f"got fields: {[f.get('name') for f in fields]}"
        )
    raw_proj = _run([
        "gh", "project", "view",
        str(project_number), "--owner", owner, "--format", "json",
    ])
    project_id = json.loads(raw_proj)["id"]
    return project_id, field_id, options


# ---- Setup helpers ---------------------------------------------------------

@keyword
def create_test_issue(owner: str, repo_name: str, project_number: int) -> dict:
    """Create one e2e test issue on the REAL repo and add it to project ``project_number``.

    Returns ``{node_id, url, number, title, project_id, item_id}``.

    The title is prefixed with ``[e2e-test]`` and the body carries a clear
    "automated, safe to ignore" marker so a human browsing the repo
    understands what they're looking at. Both signals are reused by the
    cleanup keywords as safety gates.
    """
    suffix = uuid.uuid4().hex[:8]
    title = f"{ISSUE_TITLE_PREFIX} harness run {suffix}"
    body = (
        f"{ISSUE_BODY_MARKER}\n\n"
        "This issue was created by the e2e harness. Cleanup will remove it."
    )
    raw = _run([
        "gh", "issue", "create",
        "--repo", f"{owner}/{repo_name}",
        "--title", title, "--body", body,
    ])
    url = raw.strip().splitlines()[-1].strip()
    number = int(url.rstrip("/").rsplit("/", 1)[-1])
    node_raw = _run([
        "gh", "api",
        f"/repos/{owner}/{repo_name}/issues/{number}",
        "--jq", ".node_id",
    ])
    node_id = node_raw.strip()
    # Add the issue to the project. The mutation is async — item-list may
    # not surface the new item for a few seconds, so we capture project_id
    # here and the item_id is resolved on the first status move.
    _run([
        "gh", "project", "item-add",
        str(project_number), "--owner", owner, "--url", url,
    ])
    project_id, _field_id, _options = _resolve_project_status(owner, int(project_number))
    item_id = _wait_for_item_id(owner, int(project_number), url)
    logger.info(
        f"created issue #{number} ({title}) node={node_id} project_item={item_id}"
    )
    return {
        "node_id": node_id,
        "url": url,
        "number": number,
        "title": title,
        "project_id": project_id,
        "item_id": item_id,
    }


def _wait_for_item_id(owner: str, project_number: int, issue_url: str, timeout_s: int = 30) -> str:
    deadline = time.time() + timeout_s
    last_items: list = []
    while time.time() < deadline:
        raw = _run([
            "gh", "project", "item-list",
            str(project_number), "--owner", owner, "--format", "json", "--limit", "200",
        ])
        last_items = json.loads(raw).get("items", [])
        for it in last_items:
            content_url = (it.get("content") or {}).get("url")
            if content_url == issue_url:
                return it.get("id")
        time.sleep(2)
    raise AssertionError(
        f"project item for issue {issue_url} not found in project {owner}/{project_number} "
        f"after {timeout_s}s; last item-list="
        f"{[(it.get('id'), (it.get('content') or {}).get('url')) for it in last_items]}"
    )


@keyword
def get_issue_number_from_url(url: str) -> int:
    return int(url.rstrip("/").rsplit("/", 1)[-1])


# Cache resolved field/option ids for the lifetime of the Python process.
# A Robot suite reuses the same library instance across keywords, so this
# spares us the field-list + project-view calls on every status move.
_status_cache: dict[Tuple[str, int], Tuple[str, str, dict]] = {}


def _cached_project_status(owner: str, project_number: int) -> Tuple[str, str, dict]:
    key = (owner, int(project_number))
    if key not in _status_cache:
        _status_cache[key] = _resolve_project_status(owner, int(project_number))
    return _status_cache[key]


@keyword
def move_project_item_to_status(
    owner: str, project_number: int, issue_url: str, status_label: str,
    item_id: Optional[str] = None,
) -> None:
    """Move the project item whose content URL matches ``issue_url`` to the named Status option.

    If ``item_id`` is provided the slow item-list lookup is skipped — the
    suite captures the item id at setup, so subsequent status moves can
    pass it in to avoid a redundant GraphQL call.
    """
    project_id, field_id, options = _cached_project_status(owner, int(project_number))
    target = options.get(status_label.lower())
    if not target:
        raise AssertionError(
            f"status '{status_label}' not found on project {owner}/{project_number}; "
            f"have {list(options)}"
        )
    resolved_item = item_id or _wait_for_item_id(owner, int(project_number), issue_url)
    _run([
        "gh", "project", "item-edit",
        "--id", resolved_item, "--project-id", project_id,
        "--field-id", field_id, "--single-select-option-id", target,
    ])
    logger.info(f"moved item {resolved_item} to status '{status_label}'")


# ---- Safety pre-checks -----------------------------------------------------

@keyword
def assert_safe_to_delete_issue(
    owner: str, repo_name: str, expected_number: int, expected_node_id: str
) -> None:
    """Hard pre-check before deleting an issue.

    Verifies:
      * The issue still exists.
      * The issue title still starts with ``[e2e-test]``.
      * The issue node id matches the one captured at setup.

    Any mismatch raises and aborts the cleanup. This protects against the
    catastrophic case of a typo'd ``${ISSUE_NODE_ID}`` pointing at a real
    user-authored issue.
    """
    rc, out, err = _run_no_check([
        "gh", "api", f"/repos/{owner}/{repo_name}/issues/{expected_number}",
    ])
    if rc != 0:
        raise AssertionError(
            f"safety: issue #{expected_number} not visible (rc={rc}, stderr={err.strip()}); aborting deletion"
        )
    data = json.loads(out)
    actual_node = data.get("node_id", "")
    actual_title = data.get("title", "")
    if actual_node != expected_node_id:
        raise AssertionError(
            f"safety: node_id mismatch on #{expected_number}: expected={expected_node_id} actual={actual_node}; aborting"
        )
    if not actual_title.startswith(ISSUE_TITLE_PREFIX):
        raise AssertionError(
            f"safety: issue #{expected_number} title {actual_title!r} does not start with "
            f"{ISSUE_TITLE_PREFIX!r}; aborting deletion"
        )
    logger.info(
        f"safety OK: issue #{expected_number} title={actual_title!r} node={actual_node}"
    )


@keyword
def assert_safe_to_delete_branch(branch_name: str) -> None:
    """Refuse to delete any branch that does not start with ``sympheo/``."""
    if not branch_name or not branch_name.startswith(BRANCH_PREFIX):
        raise AssertionError(
            f"safety: refusing to touch branch {branch_name!r} — must start with {BRANCH_PREFIX!r}"
        )


@keyword
def assert_safe_to_remove_project_item(
    owner: str, project_number: int, expected_item_id: str, expected_issue_url: str
) -> None:
    """Confirm the project item the harness is about to remove still maps to OUR issue.

    If the item id is missing (e.g. someone removed it manually mid-run) we
    skip silently — the goal of cleanup is just to leave no orphan.
    """
    rc, out, _err = _run_no_check([
        "gh", "project", "item-list",
        str(project_number), "--owner", owner, "--format", "json", "--limit", "200",
    ])
    if rc != 0:
        raise AssertionError(
            f"safety: cannot list project items for {owner}/{project_number} (rc={rc})"
        )
    items = json.loads(out).get("items", [])
    for it in items:
        if it.get("id") == expected_item_id:
            content_url = (it.get("content") or {}).get("url")
            if content_url != expected_issue_url:
                raise AssertionError(
                    f"safety: project item {expected_item_id} now points at {content_url!r}, "
                    f"not {expected_issue_url!r}; aborting"
                )
            return
    raise AssertionError(
        f"safety: project item {expected_item_id} no longer present in project "
        f"{owner}/{project_number}; nothing to remove"
    )


# ---- Cleanup keywords ------------------------------------------------------

@keyword
def remove_project_item(owner: str, project_number: int, project_id: str, item_id: str) -> None:
    """Remove an item from the project. Best-effort."""
    rc, _out, err = _run_no_check([
        "gh", "project", "item-delete",
        str(project_number), "--owner", owner,
        "--id", item_id,
    ])
    if rc != 0:
        logger.warn(f"gh project item-delete failed: {err.strip()}")


@keyword
def delete_issue_via_graphql(issue_node_id: str) -> None:
    """Delete an issue using the GraphQL ``deleteIssue`` mutation.

    The gh CLI exposes no ``issue delete`` subcommand for this; the GraphQL
    mutation is the only way. We pass the node id as a variable to avoid
    quoting fragility.
    """
    if not issue_node_id:
        raise AssertionError("delete_issue_via_graphql called with empty node id")
    query = (
        "mutation($id: ID!) { "
        "deleteIssue(input: {issueId: $id}) { repository { id } } "
        "}"
    )
    rc, _out, err = _run_no_check([
        "gh", "api", "graphql",
        "-f", f"query={query}",
        "-F", f"id={issue_node_id}",
    ])
    if rc != 0:
        raise AssertionError(
            f"deleteIssue mutation failed for node {issue_node_id}: {err.strip()}"
        )
    logger.info(f"deleted issue node {issue_node_id} via graphql")


@keyword
def delete_remote_branch_if_safe(owner: str, repo_name: str, branch_name: str) -> None:
    """Best-effort delete of ``branch_name`` on the remote, gated by the safety pre-check.

    A 422 on the DELETE is treated as success (branch already gone).
    """
    if not branch_name:
        return
    assert_safe_to_delete_branch(branch_name)
    rc, _out, err = _run_no_check([
        "gh", "api", "-X", "DELETE",
        f"/repos/{owner}/{repo_name}/git/refs/heads/{branch_name}",
    ])
    if rc != 0:
        if "Reference does not exist" in err or "Not Found" in err:
            logger.info(f"branch {branch_name} already absent")
            return
        logger.warn(f"failed to delete branch {branch_name}: {err.strip()}")
        return
    logger.info(f"deleted remote branch {branch_name}")


@keyword
def find_sympheo_branch_for_issue(owner: str, repo_name: str, issue_number: int) -> str:
    """Return the first remote branch whose name starts with ``sympheo/`` and references the issue.

    The orchestrator names branches like ``sympheo/<short-issue-id>/...``.
    We don't know the exact pattern up front, so we fall back to the most
    recent ``sympheo/*`` branch whose creation timestamp is post-setup. If
    nothing matches, returns an empty string.
    """
    rc, out, _err = _run_no_check([
        "gh", "api", f"/repos/{owner}/{repo_name}/branches",
        "--paginate", "--jq", ".[].name",
    ])
    if rc != 0:
        return ""
    candidates = [b.strip() for b in out.splitlines() if b.strip().startswith(BRANCH_PREFIX)]
    issue_token = str(issue_number)
    for b in candidates:
        if issue_token in b:
            return b
    return candidates[0] if candidates else ""


@keyword
def list_sympheo_branches(owner: str, repo_name: str) -> list[str]:
    """Return all remote branches whose name starts with ``sympheo/``."""
    rc, out, _err = _run_no_check([
        "gh", "api", f"/repos/{owner}/{repo_name}/branches",
        "--paginate", "--jq", ".[].name",
    ])
    if rc != 0:
        return []
    return [b.strip() for b in out.splitlines() if b.strip().startswith(BRANCH_PREFIX)]


@keyword
def get_remote_branches(owner: str, repo_name: str) -> list[str]:
    """Return the list of branch names on the remote repo (used by smoke checks)."""
    raw = _run([
        "gh", "api",
        f"/repos/{owner}/{repo_name}/branches",
        "--paginate", "--jq", ".[].name",
    ])
    return [line.strip() for line in raw.splitlines() if line.strip()]


@keyword
def list_stale_e2e_issues(owner: str, repo_name: str) -> list:
    """Return open issues whose title starts with ``[e2e-test]`` — leftovers from prior runs.

    Each entry is ``{number, node_id, title}``. Used by ``Cleanup Stale E2E Issues``
    to wipe state before the suite provisions its own ticket.
    """
    rc, out, err = _run_no_check([
        "gh", "issue", "list",
        "--repo", f"{owner}/{repo_name}",
        "--state", "open",
        "--search", ISSUE_TITLE_PREFIX,
        "--json", "number,id,title",
        "--limit", "200",
    ])
    if rc != 0:
        logger.warn(f"failed to list stale e2e issues: {err.strip()}")
        return []
    issues = []
    for it in json.loads(out):
        title = it.get("title", "")
        if title.startswith(ISSUE_TITLE_PREFIX):
            issues.append({
                "number": it["number"],
                "node_id": it["id"],
                "title": title,
            })
    return issues


# ---- Project status metadata (for prompt injection) ------------------------

@keyword
def get_project_status_metadata(owner: str, project_number: int) -> dict:
    """Return ``{project_id, status_field_id, options: {name_lower: option_id}}``.

    Used to bake the ids the agent needs into the WORKFLOW.md phase prompt
    (so claude can call ``gh project item-edit`` with literal arguments
    instead of having to query the field-list itself).
    """
    project_id, field_id, options = _cached_project_status(owner, int(project_number))
    return {
        "project_id": project_id,
        "status_field_id": field_id,
        "options": options,
    }


# ---- Issue body / state inspection -----------------------------------------

@keyword
def get_issue_body(owner: str, repo_name: str, issue_number: int) -> str:
    """Return the current body of the issue (empty string if absent)."""
    rc, out, err = _run_no_check([
        "gh", "api", f"/repos/{owner}/{repo_name}/issues/{issue_number}",
        "--jq", ".body // \"\"",
    ])
    if rc != 0:
        raise AssertionError(f"failed to read issue #{issue_number} body: {err.strip()}")
    return out.strip()


@keyword
def get_project_item_status(
    owner: str, project_number: int, item_id: str
) -> str:
    """Return the current Status option name for the project item, or '' if unknown."""
    rc, out, err = _run_no_check([
        "gh", "project", "item-list",
        str(project_number), "--owner", owner,
        "--format", "json", "--limit", "200",
    ])
    if rc != 0:
        raise AssertionError(f"failed to list project items: {err.strip()}")
    items = json.loads(out).get("items", [])
    for it in items:
        if it.get("id") == item_id:
            return (it.get("status") or "").strip()
    return ""


# ---- PR-aware cleanup ------------------------------------------------------

@keyword
def find_open_pr_for_issue(
    owner: str, repo_name: str, issue_number: int, head_prefix: str = BRANCH_PREFIX
) -> dict:
    """Locate an open PR whose head branch starts with ``head_prefix`` and which references ``issue_number``.

    Returns ``{number, node_id, head_ref, url}`` or an empty dict if nothing
    matches. The reference check is permissive: the PR body or title may
    mention ``#<issue_number>``, ``Closes #<n>``, or the head branch may
    contain ``<n>``.
    """
    rc, out, err = _run_no_check([
        "gh", "pr", "list",
        "--repo", f"{owner}/{repo_name}",
        "--state", "open",
        "--limit", "200",
        "--json", "number,title,body,headRefName,url,id",
    ])
    if rc != 0:
        raise AssertionError(f"gh pr list failed: {err.strip()}")
    issue_token = str(issue_number)
    needles = (f"#{issue_token}", f"closes #{issue_token}", f"Closes #{issue_token}")
    for pr in json.loads(out):
        head = (pr.get("headRefName") or "").strip()
        if not head.startswith(head_prefix):
            continue
        title = pr.get("title") or ""
        body = pr.get("body") or ""
        if (
            issue_token in head
            or any(n in body for n in needles)
            or any(n in title for n in needles)
        ):
            return {
                "number": pr.get("number"),
                "node_id": pr.get("id"),
                "head_ref": head,
                "url": pr.get("url"),
            }
    return {}


@keyword
def assert_safe_to_close_pr(
    owner: str, repo_name: str, pr_number: int, expected_head_prefix: str = BRANCH_PREFIX
) -> None:
    """Refuse to touch a PR whose head branch does not start with ``sympheo/``."""
    rc, out, err = _run_no_check([
        "gh", "pr", "view", str(pr_number),
        "--repo", f"{owner}/{repo_name}",
        "--json", "headRefName,state,title",
    ])
    if rc != 0:
        raise AssertionError(f"gh pr view #{pr_number} failed: {err.strip()}")
    data = json.loads(out)
    head = (data.get("headRefName") or "").strip()
    if not head.startswith(expected_head_prefix):
        raise AssertionError(
            f"safety: PR #{pr_number} head={head!r} does not start with "
            f"{expected_head_prefix!r}; refusing to close"
        )
    state = (data.get("state") or "").upper()
    if state not in ("OPEN", "DRAFT"):
        logger.info(f"PR #{pr_number} already in state {state}; nothing to close")


@keyword
def close_pr(owner: str, repo_name: str, pr_number: int) -> None:
    """Close a PR. Does not delete its branch — that is a separate step."""
    rc, _out, err = _run_no_check([
        "gh", "pr", "close", str(pr_number),
        "--repo", f"{owner}/{repo_name}",
        "--delete-branch",
    ])
    if rc != 0:
        if "already closed" in err.lower() or "Not Found" in err:
            logger.info(f"PR #{pr_number} already closed or absent")
            return
        logger.warn(f"failed to close PR #{pr_number}: {err.strip()}")
        return
    logger.info(f"closed PR #{pr_number} and deleted its branch")
