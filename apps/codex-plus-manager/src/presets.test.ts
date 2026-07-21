import assert from "node:assert";
import { describe, it } from "node:test";
import { PRESETS, createPresetPatch } from "./presets.ts";

describe("anthropic presets", () => {
  it("includes Kimi For Coding anthropic preset", () => {
    const preset = PRESETS.find((item) => item.id === "kimi-for-coding-anthropic");
    assert.ok(preset);
    assert.strictEqual(preset.protocol, "anthropic");
    assert.strictEqual(preset.baseUrl, "https://api.kimi.com/coding");
    assert.strictEqual(preset.model, "k3");
    assert.strictEqual(preset.apiKeyUrl, "https://www.kimi.com/code/console");
    assert.deepStrictEqual(preset.modelList, [
      "k3",
      "kimi-for-coding",
      "kimi-for-coding-highspeed",
    ]);
  });

  it("creates a patch with the K3 1M model window", () => {
    const preset = PRESETS.find((item) => item.id === "kimi-for-coding-anthropic");
    assert.ok(preset);

    const patch = createPresetPatch(preset);

    assert.strictEqual(patch.modelList, "k3\nkimi-for-coding\nkimi-for-coding-highspeed");
    assert.strictEqual(patch.modelWindows, '{"k3":"1M"}');
  });
});
