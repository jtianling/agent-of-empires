"""Exercise managed worktree config sync without a real tmux server."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="aoe-worktree-sync-") as directory:
        root = Path(directory).resolve()
        bin_dir = root / "bin"
        bin_dir.mkdir()
        tmux = bin_dir / "tmux"
        tmux.write_text("#!/bin/sh\nexit 1\n")
        tmux.chmod(0o700)
        env = {
            "HOME": str(root),
            "XDG_CONFIG_HOME": str(root / "config"),
            "PATH": str(bin_dir) + os.pathsep + os.defpath,
            "TMPDIR": str(root),
            "GIT_AUTHOR_NAME": "Fixture",
            "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
            "GIT_COMMITTER_NAME": "Fixture",
            "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
        }
        context = root / "manager.json"

        def call(*args, success=True):
            result = subprocess.run(
                [str(binary), "managed", "--context", str(context), *args],
                env=env, text=True, capture_output=True, timeout=30,
            )
            output = json.loads(result.stdout)
            assert output["ok"] == success, (args, output, result.stderr)
            assert (result.returncode == 0) == success, result.stderr
            return output

        repo = root / "repo"
        repo.mkdir()

        def git(*args):
            result = subprocess.run(
                ["git", "-C", str(repo), *args], env=env,
                text=True, capture_output=True, timeout=30,
            )
            assert result.returncode == 0, result.stderr
            return result.stdout

        git("init")
        (repo / "README").write_text("tracked\n")
        (repo / ".gitignore").write_text(
            ".agents/\n.codex/\n.claude/\nAGENTS.md\n.mcp.json\n"
        )
        git("add", "README", ".gitignore")
        git("commit", "-m", "Fixture")
        for agent in (".agents", ".codex", ".claude"):
            skill = repo / agent / "skills" / "fixture"
            skill.mkdir(parents=True)
            (skill / "SKILL.md").write_text("fixture skill\n")
        (repo / "AGENTS.md").write_text("fixture instructions\n")
        (repo / ".codex" / "config.toml").write_text('model = "fixture"\n')
        (repo / ".mcp.json").write_text('{"mcpServers":{}}\n')
        shared_skill = root / "shared-skill"
        shared_skill.mkdir()
        (repo / ".agents" / "skills" / "linked").symlink_to(shared_skill)
        call("init")

        worktree = root / "worktree"
        git("worktree", "add", "-b", "fixture-worktree", str(worktree))
        call("create", str(worktree), "--tool", "invalid", "--title", "invalid",
             success=False)
        assert not (worktree / ".agents").exists()
        session = call("create", str(worktree), "--tool", "codex",
                       "--title", "synced")["data"]
        for agent in (".agents", ".codex", ".claude"):
            copied = worktree / agent / "skills" / "fixture" / "SKILL.md"
            assert copied.read_text() == "fixture skill\n", copied
        link = worktree / ".agents" / "skills" / "linked"
        assert link.is_symlink() and link.readlink() == shared_skill
        assert (worktree / "AGENTS.md").read_text() == "fixture instructions\n"
        assert (worktree / ".codex" / "config.toml").read_text() == (
            'model = "fixture"\n'
        )
        stored = next(
            item for item in json.loads(next(root.rglob("sessions.json")).read_text())
            if item["id"] == session["id"]
        )
        assert stored.get("worktree_info") is None
        assert stored["primary_pane"].get("worktree") is None
        (worktree / "AGENTS.md").write_text("keep local instructions\n")
        (repo / ".agents" / "skills" / "new").mkdir()
        call("create", str(worktree), "--tool", "shell", "--title", "repeat")
        assert (worktree / "AGENTS.md").read_text() == "keep local instructions\n"
        assert not (worktree / ".agents" / "skills" / "new").exists()

        preserved = root / "preserved"
        git("worktree", "add", "-b", "preserved", str(preserved))
        (preserved / ".agents").mkdir()
        (preserved / ".agents" / "local").write_text("keep\n")
        missing = root / "missing-link-target"
        (preserved / "AGENTS.md").symlink_to(missing)
        external = root / "external"
        external.mkdir()
        (preserved / ".codex").symlink_to(external)
        call("create", str(preserved), "--tool", "shell", "--title", "preserved")
        assert (preserved / ".agents" / "local").read_text() == "keep\n"
        assert not (preserved / ".agents" / "skills").exists()
        assert (preserved / "AGENTS.md").is_symlink()
        assert not missing.exists()
        assert not (external / "config.toml").exists()

        subdir = worktree / "subdir"
        subdir.mkdir()
        call("create", str(subdir), "--tool", "shell", "--title", "subdir")
        assert list(subdir.iterdir()) == []
        ordinary = root / "ordinary"
        ordinary.mkdir()
        call("create", str(ordinary), "--tool", "shell", "--title", "ordinary")
        assert list(ordinary.iterdir()) == []
        unsupported = root / "unsupported-git"
        unsupported.mkdir()
        git("init", "--object-format=sha256", str(unsupported))
        call("create", str(unsupported), "--tool", "shell", "--title", "unsupported")
        assert [p.name for p in unsupported.iterdir()] == [".git"]
        call("create", str(repo), "--tool", "shell", "--title", "main")
        assert not (repo / ".agents" / "skills" / "fixture" / ".agents").exists()
        call("remove", session["id"])
        assert (worktree / "README").read_text() == "tracked\n"
        assert worktree.as_posix() in git("worktree", "list", "--porcelain")
        print(json.dumps({"status": "PASS", "checks": [
            "copy_skills", "preserve_symlinks", "preserve_existing",
            "no_external_writes", "invalid_input", "repeat_create",
            "ordinary_paths", "unrecognized_git", "external_worktree_ownership",
            "safe_remove",
        ]}))


if __name__ == "__main__":
    main()
