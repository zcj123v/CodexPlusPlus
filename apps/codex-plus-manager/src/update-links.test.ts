import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";

const readSource = (relativePath: string) => readFile(new URL(relativePath, import.meta.url), "utf8");

describe("update and ecosystem links", () => {
  it("uses the fork for updates while preserving BigPizzaV3 ecosystem links", async () => {
    const [appSource, updateSource, adsSource, themeSource, scriptSource] = await Promise.all([
      readSource("./App.tsx"),
      readSource("../../../crates/codex-plus-core/src/update.rs"),
      readSource("../../../crates/codex-plus-core/src/ads.rs"),
      readSource("../../../crates/codex-plus-core/src/dream_skin_market.rs"),
      readSource("../../../crates/codex-plus-core/src/script_market.rs"),
    ]);

    assert.match(updateSource, /zcj123v\/CodexPlusPlus\/releases\/latest\/download\/latest\.json/);
    assert.match(appSource, /github\.com\/BigPizzaV3\/CodexPlusPlus/);
    assert.match(adsSource, /BigPizzaV3\/Ad-List/);
    assert.match(themeSource, /BigPizzaV3\/CodexPlusPlus-Themes/);
    assert.match(scriptSource, /BigPizzaV3\/CodexPlusPlusScriptMarket/);
  });

  it("presents Linux updates as downloads with a release-page fallback", async () => {
    const appSource = await readSource("./App.tsx");

    assert.match(appSource, /releaseUrl\?: string/);
    assert.match(appSource, /url: update\.releaseUrl \?\? ""/);
    assert.match(appSource, /update\.assetUrl \|\| update\.releaseUrl/);
    assert.match(appSource, /打开安装包下载/);
    assert.match(appSource, /打开 Release 页面/);
    assert.match(appSource, /正在打开下载页面…/);
  });
});
