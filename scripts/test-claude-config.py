#!/usr/bin/env python3
"""Behaviour of agent instruction checks against small repository trees."""
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

    def test_root_instruction_documents_check_links_and_cited_paths(self):
        for name in ("AGENTS.md", "CLAUDE.md", "CONTRIBUTING.md"):
            with self.subTest(document=name):
                self.write(name, "Read [guide](missing.md) and `scripts/missing.py`.\n")
                self.assertReports(f"{name}: broken link missing.md")
                self.assertReports(f"{name}: cited path scripts/missing.py does not exist")

    def test_missing_anchors_fail_in_same_document_and_relative_links(self):
        self.write("AGENTS.md", "# Guide\n[local](#missing) [other](docs/rules.md#missing)\n")
        self.write("docs/rules.md", "## Rules\n")
        self.assertEqual(self.errors(), [
            "AGENTS.md: broken anchor #missing",
            "AGENTS.md: broken anchor docs/rules.md#missing",
        ])

    def test_heading_anchors_follow_rendered_text_and_disambiguate_duplicates(self):
        self.write("docs/rules.md", "# Checks\n## Checks\n## Checks-1\n"
                   "### Use `Vec<T>` and `_snake_case` with **bold**, _emphasis_ &amp; [links](target). ###\n"
                   "## Проверки\n")
        self.write("AGENTS.md", "[one](docs/rules.md#checks) [two](docs/rules.md#checks-1)\n"
                   "[three](docs/rules.md#checks-1-1)\n"
                   "[format](docs/rules.md#use-vect-and-_snake_case-with-bold-emphasis--links)\n"
                   "[unicode](docs/rules.md#%D0%BF%D1%80%D0%BE%D0%B2%D0%B5%D1%80%D0%BA%D0%B8)\n")
        self.assertEqual(self.errors(), [])

    def test_explicit_html_anchors_and_same_document_links_resolve(self):
        self.write("AGENTS.md", '<a name="legacy"></a>\n<section id="current"></section>\n'
                   '[old](#legacy) [new](#current) [same](AGENTS.md#current)\n')
        self.assertEqual(self.errors(), [])

    def test_fenced_examples_are_not_links_citations_or_heading_targets(self):
        self.write("AGENTS.md", "# Guide\n````md\n# Sample\n[bad](missing.md)\n"
                   "`scripts/missing.py`\n```\n# Still fenced\n````\n"
                   "~~~md\n# Tilde sample\n[bad](missing.md)\n~~~\n"
                   "`[inline example](missing.md)`\n[real](#guide)\n")
        self.assertEqual(self.errors(), [])
        self.write("CLAUDE.md", "[sample](AGENTS.md#sample) [tilde](AGENTS.md#tilde-sample)\n"
                   "[nested](AGENTS.md#still-fenced)\n")
        self.assertEqual(len(self.errors()), 3)
        self.assertTrue(all("broken anchor" in error for error in self.errors()))

    def test_inline_html_examples_do_not_supply_real_anchors(self):
        self.write("AGENTS.md", '`<a id="example"></a>`\n[invalid](#example)\n')
        self.assertEqual(self.errors(), ["AGENTS.md: broken anchor #example"])

    def test_external_links_and_non_markdown_fragments_are_not_checked(self):
        self.write("AGENTS.md", "[web](https://example.invalid/guide#absent)\n"
                   "[cdn](//example.invalid/guide) [mail](mailto:author@example.invalid)\n"
                   "[source](src/lib.rs#L10)\n")
        self.write("src/lib.rs", "")
        self.assertEqual(self.errors(), [])

    def test_claude_imports_resolve_and_ignore_fenced_examples(self):
        self.write("CLAUDE.md", "@AGENTS.md\n```md\n@missing-example.md\n```\n")
        self.assertEqual(self.errors(), ["CLAUDE.md: imported file AGENTS.md does not exist"])
        self.write("AGENTS.md", "# Guide\n")
        self.assertEqual(self.errors(), [])


if __name__ == "__main__":
    unittest.main()
