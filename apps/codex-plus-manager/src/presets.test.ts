import assert from "node:assert/strict";
import { describe, it, test } from "node:test";

import { PRESETS, createPresetPatch } from "./presets.ts";

describe("provider presets", () => {
  it("keeps MiniMax China and global credentials in separate presets", () => {
    const china = PRESETS.find((preset) => preset.id === "minimax");
    const global = PRESETS.find((preset) => preset.id === "minimax-global");

    assert.deepStrictEqual(china, {
      id: "minimax",
      name: "MiniMax (China)",
      websiteUrl: "https://platform.minimaxi.com",
      apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
      category: "cn_official",
      baseUrl: "https://api.minimaxi.com/v1",
      protocol: "chatCompletions",
      model: "MiniMax-M3",
      modelList: ["MiniMax-M3", "MiniMax-M2.7"],
    });

    assert.deepStrictEqual(global, {
      id: "minimax-global",
      name: "MiniMax (Global)",
      websiteUrl: "https://platform.minimax.io",
      apiKeyUrl: "https://platform.minimax.io/subscribe/coding-plan",
      category: "official",
      baseUrl: "https://api.minimax.io/v1",
      protocol: "chatCompletions",
      model: "MiniMax-M3",
      modelList: ["MiniMax-M3", "MiniMax-M2.7"],
    });
  });
});

test("DeepSeek preset uses the official Responses integration", () => {
  const preset = PRESETS.find((candidate) => candidate.id === "deepseek");
  assert.ok(preset);
  assert.equal(preset.baseUrl, "https://api.deepseek.com/");
  assert.equal(preset.protocol, "responses");
  assert.equal(preset.model, "deepseek-v4-flash");
  assert.deepEqual(preset.modelList, ["deepseek-v4-flash", "deepseek-v4-pro"]);
});

test("includes Kimi For Coding anthropic preset", () => {
  const preset = PRESETS.find((item) => item.id === "kimi-for-coding-anthropic");
  assert.ok(preset);
  assert.equal(preset.protocol, "anthropic");
  assert.equal(preset.baseUrl, "https://api.kimi.com/coding");
  assert.equal(preset.model, "k3");
  assert.equal(preset.apiKeyUrl, "https://www.kimi.com/code/console");
  assert.deepEqual(preset.modelList, ["k3", "kimi-for-coding", "kimi-for-coding-highspeed"]);
});

test("creates a patch with the K3 1M model window", () => {
  const preset = PRESETS.find((item) => item.id === "kimi-for-coding-anthropic");
  assert.ok(preset);

  const patch = createPresetPatch(preset);

  assert.equal(patch.modelList, "k3\nkimi-for-coding\nkimi-for-coding-highspeed");
  assert.equal(patch.modelWindows, '{"k3":"1M"}');
});
