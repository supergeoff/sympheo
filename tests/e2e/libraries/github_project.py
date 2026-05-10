"""Robot Framework helpers for GitHub Project v2 manipulations.

The `gh` CLI handles repos and basic project operations well, but moving an
item between Status options requires resolving the Status field-id and the
target option-id. This library wraps that with a thin Python layer.

All keywords assume the user is authenticated with `gh auth login` (or
`GITHUB_TOKEN` is exported) and that the resources exist.
"""
from __future__ import annotations

import json
import subprocess
import time
from typing import Optional, Tuple

from robot.api.deco import keyword
from robot.api import logger


def _run(cmd: list[str], check: bool = True) -> str:
    """Run a subprocess and return stdout. On failure, dump stderr to log."""
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if check and proc.returncode != 0:
        raise AssertionError(
            f"{' '.join(cmd)} exited {proc.returncode}\nstdout: {proc.stdout}\nstderr: {proc.stderr}"
        )
    return proc.stdout


def _gh_user() -> str:
    out = _run(["gh", "api", "user", "--jq", ".login"]).strip()
    return out


def _resolve_project_status(owner: str, project_number: int) -> Tuple[str, str, dict]:
    """Return (project_id, status_field_id, {option_name_lower: option_id})."""
    raw = _run([
        "gh", "project", "field-list",
        str(project_number), "--owner", owner, "--format", "json",
    ])
    data = json.loads(raw)
    fields = data.get("fields", data)  # newer gh wraps in {fields: [...]}; older returns list
    project_id = ""
    field_id = ""
    options: dict[str, str] = {}
    for f in fields:
        if f.get("name") == "Status" and f.get("type", "").endswith("SingleSelectField"):
            field_id = f["id"]
            for o in f.get("options", []):
                options[o["name"].lower()] = o["id"]
    if not field_id:
        raise AssertionError(f"Status field not found on project {owner}/{project_number}; got fields: {[f.get('name') for f in fields]}")
    # Project node id (PVT_*) lookup
    raw_proj = _run([
        "gh", "project", "view",
        str(project_number), "--owner", owner, "--format", "json",
    ])
    project_id = json.loads(raw_proj)["id"]
    return project_id, field_id, options


@keyword
def get_gh_user() -> str:
    """Return the authenticated GitHub login."""
    return _gh_user()


@keyword
def create_throwaway_repo(owner: str, repo_name: str) -> str:
    """Create a private repo with auto-init, return clone URL (https)."""
    _run([
        "gh", "repo", "create",
        f"{owner}/{repo_name}",
        "--private", "--add-readme",
        "--description", "sympheo e2e test (auto-generated, safe to delete)",
    ])
    # Wait briefly for replication; gh repo view will fail until then.
    deadline = time.time() + 20
    while time.time() < deadline:
        proc = subprocess.run(
            ["gh", "repo", "view", f"{owner}/{repo_name}", "--json", "url"],
            capture_output=True, text=True,
        )
        if proc.returncode == 0:
            return json.loads(proc.stdout)["url"] + ".git"
        time.sleep(1)
    raise AssertionError(f"repo {owner}/{repo_name} not visible after 20s")


@keyword
def create_project_with_statuses(owner: str, title: str) -> int:
    """Create a Project v2 (auto-includes Status field with Todo/In Progress/Done) and return its number."""
    raw = _run([
        "gh", "project", "create",
        "--owner", owner, "--title", title, "--format", "json",
    ])
    data = json.loads(raw)
    return int(data["number"])


@keyword
def add_test_issue(owner: str, repo_name: str, project_number: int, title: str, body: str) -> dict:
    """Create an issue, add it to the project, return ``{node_id, url, number}``.

    ``node_id`` is the GraphQL node id of the issue itself (``I_kwDO…``);
    sympheo uses it as ``Issue.id`` in the running map (see
    ``src/tracker/github.rs::normalize_item``). The ``url`` is needed to
    locate the project-item id (``PVTI_*``) for status updates, since
    ``gh project item-list --format json`` does NOT expose the issue node id.
    """
    raw = _run([
        "gh", "issue", "create",
        "--repo", f"{owner}/{repo_name}",
        "--title", title, "--body", body,
    ])
    # gh issue create prints the issue URL on the last line of stdout.
    url = raw.strip().splitlines()[-1].strip()
    number = int(url.rstrip("/").rsplit("/", 1)[-1])
    # Look up the issue node id via gh api.
    node_raw = _run([
        "gh", "api",
        f"/repos/{owner}/{repo_name}/issues/{number}",
        "--jq", ".node_id",
    ])
    node_id = node_raw.strip()
    # Add to project.
    _run([
        "gh", "project", "item-add",
        str(project_number), "--owner", owner, "--url", url,
    ])
    return {"node_id": node_id, "url": url, "number": number}


@keyword
def get_issue_number_from_url(url: str) -> int:
    return int(url.rstrip("/").rsplit("/", 1)[-1])


@keyword
def move_project_item_to_status(
    owner: str, project_number: int, issue_url: str, status_label: str
) -> None:
    """Move the project item whose content URL matches ``issue_url`` to the named Status option.

    We match by URL because ``gh project item-list --format json`` does not
    expose the issue's GraphQL node id — only its URL, number and repo path.
    """
    project_id, field_id, options = _resolve_project_status(owner, int(project_number))
    target = options.get(status_label.lower())
    if not target:
        raise AssertionError(
            f"status '{status_label}' not found on project {owner}/{project_number}; have {list(options)}"
        )
    # gh project mutations propagate asynchronously: item-add + item-list
    # back-to-back can return [] for ~5-10 seconds. Retry until the item is
    # visible (or fail loudly with the last observed list for diagnostics).
    item_id: Optional[str] = None
    items: list = []
    deadline = time.time() + 30
    while time.time() < deadline:
        raw = _run([
            "gh", "project", "item-list",
            str(project_number), "--owner", owner, "--format", "json", "--limit", "200",
        ])
        items = json.loads(raw).get("items", [])
        for it in items:
            content_url = (it.get("content") or {}).get("url")
            if content_url == issue_url:
                item_id = it.get("id")
                break
        if item_id:
            break
        time.sleep(2)
    if not item_id:
        raise AssertionError(
            f"project item for issue {issue_url} not found in project {owner}/{project_number} after 30s; last item-list={[(it.get('id'), (it.get('content') or {}).get('url')) for it in items]}"
        )
    _run([
        "gh", "project", "item-edit",
        "--id", item_id, "--project-id", project_id,
        "--field-id", field_id, "--single-select-option-id", target,
    ])
    logger.info(f"moved item {item_id} to status '{status_label}'")


@keyword
def delete_project(owner: str, project_number: int) -> None:
    proc = subprocess.run(
        ["gh", "project", "delete", str(project_number), "--owner", owner],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        logger.warn(f"gh project delete failed: {proc.stderr}")


@keyword
def delete_repo(owner: str, repo_name: str) -> None:
    proc = subprocess.run(
        ["gh", "repo", "delete", f"{owner}/{repo_name}", "--yes"],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        logger.warn(f"gh repo delete failed: {proc.stderr}")


@keyword
def get_remote_branches(owner: str, repo_name: str) -> list[str]:
    """Return the list of branch names on the remote repo."""
    raw = _run([
        "gh", "api",
        f"/repos/{owner}/{repo_name}/branches",
        "--paginate", "--jq", ".[].name",
    ])
    return [line.strip() for line in raw.splitlines() if line.strip()]


@keyword
def assert_any_branch_starts_with(owner: str, repo_name: str, prefix: str) -> str:
    """Fail unless at least one remote branch starts with `prefix`. Returns the matching branch."""
    branches = get_remote_branches(owner, repo_name)
    for b in branches:
        if b.startswith(prefix):
            return b
    raise AssertionError(
        f"no branch on {owner}/{repo_name} starts with {prefix!r}; have {branches}"
    )
