import assert from "node:assert/strict";
import test from "node:test";
import { changeTypeLabel, changeTypeSlug } from "../src/lib/changeType.ts";

test("已知英文枚举映射中文标签", () => {
  assert.equal(changeTypeLabel("added"), "新增");
  assert.equal(changeTypeLabel("supplemented"), "补充");
  assert.equal(changeTypeLabel("revised"), "修正");
  assert.equal(changeTypeLabel("overturned"), "推翻");
  assert.equal(changeTypeLabel("merged"), "合并");
  assert.equal(changeTypeLabel("validated"), "验证");
});

test("大小写与空白不影响识别", () => {
  assert.equal(changeTypeLabel("  Added "), "新增");
  assert.equal(changeTypeLabel("OVERTURNED"), "推翻");
});

test("未知值兜底为变化，不把原文抛给界面", () => {
  assert.equal(changeTypeLabel("reframed"), "变化");
  assert.equal(changeTypeLabel(""), "变化");
  assert.equal(changeTypeSlug("reframed"), "other");
});

test("slug 给 CSS 用：已知值原文、未知值 other", () => {
  assert.equal(changeTypeSlug("added"), "added");
  assert.equal(changeTypeSlug("Revised"), "revised");
  assert.equal(changeTypeSlug("新增"), "other");
});
