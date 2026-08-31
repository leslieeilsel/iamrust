import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import ts from "../apps/desktop/node_modules/typescript/lib/typescript.js";

const workspace = process.cwd();
const sourceRoot = path.join(workspace, "apps/desktop/src");
const i18nFile = path.join(sourceRoot, "lib/i18n.ts");
const ignored = new Set([
  i18nFile,
  path.join(sourceRoot, "lib/demo.ts"),
  path.join(sourceRoot, "state/chat-store.ts"),
]);

const files = walk(sourceRoot).filter(
  (file) =>
    /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) && !ignored.has(file),
);

let changed = 0;
for (const file of files) {
  const source = fs.readFileSync(file, "utf8");
  const sourceFile = ts.createSourceFile(
    file,
    source,
    ts.ScriptTarget.Latest,
    true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const replacements = [];

  visit(sourceFile);
  if (replacements.length === 0) continue;

  replacements.sort((left, right) => right.start - left.start);
  let output = source;
  for (const replacement of replacements) {
    output =
      output.slice(0, replacement.start) +
      replacement.text +
      output.slice(replacement.end);
  }
  if (!/^import \{ tr \} from /m.test(output)) {
    const imports = sourceFile.statements.filter(ts.isImportDeclaration);
    const insertAt = imports.at(-1)?.end ?? 0;
    const relative = path
      .relative(path.dirname(file), i18nFile)
      .replace(/\\/g, "/")
      .replace(/\.ts$/, "");
    const moduleName = relative.startsWith(".") ? relative : `./${relative}`;
    output = `${output.slice(0, insertAt)}\nimport { tr } from '${moduleName}';${output.slice(insertAt)}`;
  }
  fs.writeFileSync(file, output);
  changed += 1;

  function visit(node) {
    if (insideTranslationCall(node) || !isRuntimeNode(node)) {
      ts.forEachChild(node, visit);
      return;
    }
    if (
      ts.isTemplateExpression(node) &&
      containsHan(node.getText(sourceFile))
    ) {
      replacements.push({
        start: node.getStart(sourceFile),
        end: node.end,
        text: `tr(${node.getText(sourceFile)})`,
      });
      return;
    }
    if (ts.isJsxText(node) && containsHan(node.text)) {
      const value = node.text.replace(/\s+/g, " ").trim();
      if (!value) return;
      const inline = !node.text.includes("\n");
      const leading = inline && /^\s/.test(node.text) ? "{' '}" : "";
      const trailing = inline && /\s$/.test(node.text) ? "{' '}" : "";
      replacements.push({
        start: node.getStart(sourceFile),
        end: node.end,
        text: `${leading}{tr(${JSON.stringify(value)})}${trailing}`,
      });
      return;
    }
    if (
      ts.isStringLiteralLike(node) &&
      containsHan(node.text) &&
      !isPropertyName(node)
    ) {
      const call = `tr(${JSON.stringify(node.text)})`;
      replacements.push({
        start: node.getStart(sourceFile),
        end: node.end,
        text: ts.isJsxAttribute(node.parent) ? `{${call}}` : call,
      });
      return;
    }
    ts.forEachChild(node, visit);
  }
}

console.log(`Localized ${changed} source files.`);

function walk(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(directory, entry.name);
    return entry.isDirectory() ? walk(target) : [target];
  });
}

function containsHan(value) {
  return /\p{Script=Han}/u.test(value);
}

function isRuntimeNode(node) {
  for (let parent = node.parent; parent; parent = parent.parent) {
    if (ts.isFunctionLike(parent)) return true;
    if (ts.isSourceFile(parent)) return false;
  }
  return false;
}

function insideTranslationCall(node) {
  for (
    let parent = node.parent;
    parent && !ts.isSourceFile(parent);
    parent = parent.parent
  ) {
    if (
      ts.isCallExpression(parent) &&
      ts.isIdentifier(parent.expression) &&
      parent.expression.text === "tr"
    ) {
      return true;
    }
  }
  return false;
}

function isPropertyName(node) {
  const parent = node.parent;
  return (
    (ts.isPropertyAssignment(parent) ||
      ts.isPropertySignature(parent) ||
      ts.isMethodDeclaration(parent) ||
      ts.isMethodSignature(parent)) &&
    parent.name === node
  );
}
