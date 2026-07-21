import assert from "node:assert";
import { describe, it } from "node:test";
import { PRESETS } from "./presets.ts";

describe("anthropic presets", () => {
  it("includes Kimi For Coding anthropic preset", () => {
    const preset = PRESETS.find((item) => item.id === "kimi-for-coding-anthropic");
    assert.ok(preset);
    assert.strictEqual(preset.protocol, "anthropic");
    assert.strictEqual(preset.baseUrl, "https://api.kimi.com/coding");
    assert.strictEqual(preset.model, "k3");
    assert.ok(preset.modelList?.includes("kimi-for-coding"));
  });
});
