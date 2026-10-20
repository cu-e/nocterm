#!/usr/bin/env python3
"""Behaviour of scripts/check-claude-config.py against small .claude trees."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "check-claude-config.py"
SPEC = importlib.util.spec_from_file_location("check_claude_config", SCRIPT)
config = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(config)

PROCEDURE = "---\nname: {name}\ndescription: Does one step.\nuser-invocable: false\n---\n\nBody.\n"
WORKFLOW = "---\nname: {name}\ndescription: Runs steps.\nargument-hint: \"<task>\"\n---\n\nTask: $ARGUMENTS\n"


class Tree(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.skill("placement", PROCEDURE)
        self.skill("refactor", WORKFLOW)
        self.write(".claude/agents/architect.md", "---\nname: architect\nskills:\n  - placement\n---\n\nPlan.\n")

    def tearDown(self):
        self.directory.cleanup()

    def write(self, path, text):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def skill(self, name, template, body=""):
        self.write(f".claude/skills/{name}/SKILL.md", template.format(name=name) + body)

    def errors(self):
        return config.check(self.root)

    def assertReports(self, fragment):
        self.assertTrue(any(fragment in error for error in self.errors()), self.errors())

    def test_coherent_configuration_passes(self):
        self.assertEqual(self.errors(), [])

    def test_skill_name_matches_its_directory(self):
        self.write(".claude/skills/gate/SKILL.md", PROCEDURE.format(name="gates"))
        self.assertReports("differs from directory")

    def test_descriptions_fit_the_listing_limit(self):
        self.write(".claude/skills/gate/SKILL.md", PROCEDURE.format(name="gate").replace("Does one step.", "x" * 1537))
        self.assertReports("longer than 1536")

    def test_procedure_skills_are_hidden_and_workflows_take_arguments(self):
        self.skill("gate", PROCEDURE.replace("user-invocable: false\n", ""))
        self.skill("split", WORKFLOW.replace("$ARGUMENTS", "the task"))
        self.assertReports("gate/SKILL.md: procedure skill must set user-invocable: false")
        self.assertReports("split/SKILL.md: argument-hint without $ARGUMENTS")

    def test_agents_preload_only_existing_skills(self):
        self.write(".claude/agents/reviewer.md", "---\nname: reviewer\nskills: placement, review\n---\n")
        self.assertEqual(self.errors(), [".claude/agents/reviewer.md: preloads unknown skill review"])

    def test_links_and_cited_paths_resolve_outside_code_samples(self):
        self.write("docs/adr/README.md", "index\n")
        self.skill("docs-adr", PROCEDURE, "See [index](../../../docs/adr/README.md) and `docs/adr/README.md`.\n"
                   "Plans go in ignored `.claude/plans/`, absent from a fresh checkout.\n"
                   "```\n| [NNNN](NNNN-slug.md) |\n```\nTemplate `crates/<name>/src/lib.rs`.\n")
        self.assertEqual(self.errors(), [])
        self.skill("gate", PROCEDURE, "Run `scripts/missing.py` and read [notes](notes.md).\n")
        self.assertReports("cited path scripts/missing.py does not exist")
        self.assertReports("broken link notes.md")

    def test_hook_commands_name_existing_scripts(self):
        command = 'python3 "$CLAUDE_PROJECT_DIR/.claude/hooks/guard.py"'
        settings = {"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": command}]}]}}
        self.write(".claude/settings.json", json.dumps(settings))
        self.assertReports("hook script .claude/hooks/guard.py does not exist")
        self.write(".claude/hooks/guard.py", "")
        self.assertEqual(self.errors(), [])


if __name__ == "__main__":
    unittest.main()
