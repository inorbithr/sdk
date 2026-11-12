"""The passes-on-init test: the server is started the way a host starts it (a child process over
stdio) and spoken to by the official client. It checks the 2026-07-28 revision (no handshake:
`server/discover`), that a client of the older revision still gets its handshake, the tool
list and its order, a tool call and its structured result, a refused bad argument, and the
rules every tool must meet."""

import asyncio
import json
import sys
import unittest

from mcp import Client, StdioServerParameters

SERVER = StdioServerParameters(command=sys.executable, args=["-m", "@@MODULE@@"])


def run(coro):
    return asyncio.run(asyncio.wait_for(coro, timeout=60))


class ServerTest(unittest.TestCase):
    def test_discover_answers_on_the_2026_07_28_revision(self):
        async def go():
            async with Client(SERVER, mode="auto") as client:
                self.assertEqual(client.protocol_version, "2026-07-28")
                self.assertEqual(client.server_info.name, "@@NAME@@")

        run(go())

    def test_a_client_of_the_older_revision_still_gets_its_handshake(self):
        async def go():
            # `initialize` was removed in 2026-07-28; the SDK still answers it for 2025-11-25 hosts.
            async with Client(SERVER, mode="legacy") as client:
                self.assertEqual(client.protocol_version, "2025-11-25")
                self.assertEqual(client.server_info.name, "@@NAME@@")

        run(go())

    def test_tools_list_names_the_tool_and_comes_in_the_same_order_twice(self):
        async def go():
            async with Client(SERVER) as client:
                first = await client.list_tools()
                second = await client.list_tools()
                self.assertEqual([t.name for t in first.tools], ["word_count"])
                self.assertEqual([t.name for t in first.tools], [t.name for t in second.tools])
                # Lists say how long a client may keep them (2026-07-28).
                dumped = first.model_dump(by_alias=True, exclude_none=True)
                self.assertIn("ttlMs", dumped)
                self.assertIn("cacheScope", dumped)

        run(go())

    def test_a_tool_call_returns_the_structured_result(self):
        async def go():
            async with Client(SERVER) as client:
                result = await client.call_tool("word_count", {"text": "one two  three"})
                self.assertFalse(result.is_error)
                self.assertEqual(result.structured_content, {"words": 3, "characters": 14})

        run(go())

    def test_a_bad_argument_is_refused_and_the_tool_does_not_run(self):
        async def go():
            async with Client(SERVER) as client:
                too_long = await client.call_tool("word_count", {"text": "x" * 10_001})
                self.assertTrue(too_long.is_error)
                missing = await client.call_tool("word_count", {})
                self.assertTrue(missing.is_error)

        run(go())

    def test_the_resource_is_listed_and_can_be_read(self):
        async def go():
            async with Client(SERVER) as client:
                listed = await client.list_resources()
                self.assertEqual([str(r.uri) for r in listed.resources], ["about://server"])
                read = await client.read_resource("about://server")
                self.assertIn("counts words", read.contents[0].text)

        run(go())

    def test_every_tool_has_a_description_an_annotation_and_bounded_inputs(self):
        async def go():
            async with Client(SERVER) as client:
                for tool in (await client.list_tools()).tools:
                    self.assertTrue(tool.description, f"{tool.name}: no description")
                    a = tool.annotations
                    self.assertTrue(
                        a is not None and a.read_only_hint is not None,
                        f"{tool.name}: say whether it only reads (readOnlyHint)",
                    )
                    if not a.read_only_hint:
                        self.assertIsNotNone(a.destructive_hint, f"{tool.name}: say if it destroys (destructiveHint)")
                    props = tool.input_schema.get("properties", {})
                    for name, schema in props.items():
                        if schema.get("type") == "string":
                            self.assertTrue(
                                {"maxLength", "enum", "pattern"} & schema.keys(),
                                f"{tool.name}.{name}: bound the string (maxLength, enum or pattern)",
                            )
                        if schema.get("type") in ("integer", "number"):
                            self.assertTrue(
                                {"minimum", "maximum"} & schema.keys(),
                                f"{tool.name}.{name}: bound the number (minimum, maximum)",
                            )
                    json.dumps(tool.input_schema)

        run(go())


if __name__ == "__main__":
    unittest.main()
