import { describe, expect, it } from "vitest";
import { candidateLocalPathsFromText } from "./localPathIntent";
import {
  approvedLocalFileContextReady,
  approvedLocalFilesContextReady,
  approvedLocalFilesPrompt,
  approvedLocalFilePrompt,
  directLocalFileReadPath,
  directLocalFileReadPaths,
} from "./directLocalFileRead";

const path = "/Users/example/Desktop/Quarterly Review.pdf";
const escapedICloudPath = String.raw`/Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU/oomu-profile.jpeg`;
const iCloudPath = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU/oomu-profile.jpeg";
const scenarioThreeFile = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt";
const escapedScenarioThreeFile = String.raw`/Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt`;

function detect(message: string) {
  return directLocalFileReadPath(message, candidateLocalPathsFromText(message));
}

describe("direct local file reads", () => {
  it.each([
    `Can you view this file? '${path}'`,
    `Tell me what you see with this image '${path}'`,
    `Look at this image '${path}'`,
    `What do you see in this image? '${path}'`,
    `With '${path}', tell me what you see in the image.`,
    `Kannst du diese Datei öffnen? '${path}'`,
    `¿Puedes abrir este archivo? '${path}'`,
    `Peux-tu ouvrir ce fichier ? '${path}'`,
    `Bisakah Anda membuka file ini? '${path}'`,
    `このファイルを開いてください: '${path}'`,
    `Pode abrir este arquivo? '${path}'`,
    `Можешь открыть этот файл? '${path}'`,
    `Можеш відкрити цей файл? '${path}'`,
    `Bạn có thể mở tệp này không? '${path}'`,
    `请查看这个文件：'${path}'`,
    `請檢視這個檔案：'${path}'`,
  ])("recognizes an explicit read request before inference: %s", (message) => {
    expect(detect(message)).toBe(path);
  });

  it("uses the permission prompt, rather than a hidden verb list, to resolve a safe file reference", () => {
    expect(detect(`This image is here: '${path}'`)).toBe(path);
  });

  it("normalizes a Finder or shell-escaped iCloud path before permission", () => {
    const prompt = `Tell me what you see in this image: ${escapedICloudPath}`;
    expect(candidateLocalPathsFromText(prompt)).toEqual([iCloudPath]);
    expect(detect(prompt)).toBe(iCloudPath);
    expect(approvedLocalFilePrompt(prompt, iCloudPath)).toBe(
      "Tell me what you see in this image: [approved file]",
    );
  });

  it("normalizes an escaped iCloud path inside quotes", () => {
    const prompt = `Look at this image: "${escapedICloudPath}"`;
    expect(candidateLocalPathsFromText(prompt)).toEqual([iCloudPath]);
    expect(detect(prompt)).toBe(iCloudPath);
  });

  it("keeps the published bounded read-and-summary on the approved direct-read path", () => {
    const prompt = `Read \`${escapedScenarioThreeFile}\` in my testing folder and summarize only the stated facts in exactly three bullets. Do not recommend a vendor and do not use the internet.`;

    expect(candidateLocalPathsFromText(prompt)).toEqual([scenarioThreeFile]);
    expect(detect(prompt)).toBe(scenarioThreeFile);
    expect(approvedLocalFilePrompt(prompt, scenarioThreeFile)).toBe(
      "Read `[approved file]` in my testing folder and summarize only the stated facts in exactly three bullets. Do not recommend a vendor and do not use the internet.",
    );
  });

  it("passes a directory correction to native approval-gated target classification", () => {
    const directory = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data";
    const prompt = `The correct file path to the JSON files is actually here: ${directory}`;

    expect(candidateLocalPathsFromText(prompt)).toEqual([directory]);
    expect(detect(prompt)).toBe(directory);
  });

  it.each([
    "/Users/example/.config",
    "/Users/example/Release.v1",
    "/Users/example/archive.json",
    "/Users/example/Archive.json Data/Source Files",
    "/Users/example/LICENSE",
  ])("never guesses target type from the path spelling: %s", (target) => {
    expect(detect(`Inspect '${target}'`)).toBe(target);
  });

  it.each([
    "Can you view this file? file:///Users/example/Desktop/Quarterly%20Review.pdf",
    `Can you view this file? <${path}>`,
  ])("normalizes local file references before permission: %s", (message) => {
    expect(detect(message)).toBe(path);
  });

  it.each([
    `Can you review this sentence: '${path}' is only an example path.`,
    `Can you see why this error mentions ${path}?`,
    `Open ${path} and save a copy.`,
    `Ask Alex to review ${path} before answering.`,
    "Can you view this file? https://example.test/report.png",
    "Can you view this file? file://remote-host/Users/example/Desktop/report.pdf",
  ])("rejects indirect, mixed, or non-local references: %s", (message) => {
    expect(detect(message)).toBeNull();
  });

});

describe("direct local file read safety", () => {
  it("opens explicitly named evidence before a read-only comparison", () => {
    expect(detect(`Compare the architecture described by '${path}' with our current approach and recommend a rollout.`)).toBe(path);
  });

  it("honors an explicit instruction not to open a path", () => {
    expect(detect(`Compare '${path}' with our current approach, but do not open the path.`)).toBeNull();
  });

  it("treats command words inside a filename as inert", () => {
    const commandNamedPath = "/Users/example/Desktop/delete-forecast.png";
    expect(detect(`Can you view this file? '${commandNamedPath}'`)).toBe(commandNamedPath);
    expect(detect(`View '${commandNamedPath}' and then delete it.`)).toBeNull();
  });

  it("removes the host path before approved context reaches a model", () => {
    const prompt = `Can you view '${path}'?`;
    const safePrompt = approvedLocalFilePrompt(prompt, path);
    expect(safePrompt).toContain("[approved file]");
    expect(safePrompt).not.toContain("/Users/example/Desktop");
  });

  it("removes an encoded file URI before approved context reaches a model", () => {
    const prompt = "Can you view file:///Users/example/Desktop/Quarterly%20Review.pdf?";
    const safePrompt = approvedLocalFilePrompt(prompt, path);
    expect(safePrompt).toContain("[approved file]");
    expect(safePrompt).not.toContain("file://");
    expect(safePrompt).not.toContain("/Users/example/Desktop");
  });

  it("keeps verified approved context valid after safe attachment cloning", () => {
    const approved = {
      name: "Quarterly Review.pdf",
      mime_type: "text/plain",
      byte_count: 24,
      approved_file_receipt: {
        payload: "signed-payload",
        signature: {
          public_key: "public-key",
          signature: "signature",
          payload_hash: "payload-hash",
          signed_at_ms: 1,
        },
      },
    };
    expect(approvedLocalFileContextReady(approved, [{ ...approved }])).toBe(true);
    expect(approvedLocalFileContextReady(approved, [{
      ...approved,
      approved_file_receipt: {
        ...approved.approved_file_receipt,
        payload: "different-payload",
      },
    }])).toBe(false);
  });

  it("requires one matching receipt attachment for every approved file", () => {
    const approved = {
      name: "Quarterly Review.pdf",
      mime_type: "text/plain",
      byte_count: 24,
      approved_file_receipt: {
        payload: "signed-payload",
        signature: {
          public_key: "public-key",
          signature: "signature",
          payload_hash: "payload-hash",
          signed_at_ms: 1,
        },
      },
    };

    expect(approvedLocalFilesContextReady([approved, approved], [{ ...approved }])).toBe(false);
    expect(approvedLocalFilesContextReady([approved, approved], [{ ...approved }, { ...approved }])).toBe(true);
  });
});

describe("named file inside an explicit folder", () => {
  it("resolves the exact file before inference", () => {
    const folder = "/Users/example/Documents/OOMU/Projects/mock_data";
    const filePath = `${folder}/Lab_Inventory.csv`;
    const prompt = `Read the CSV file Lab_Inventory.csv located in "${folder}" and summarize how many items are listed, along with any items that have low stock.`;

    expect(candidateLocalPathsFromText(prompt)).toEqual([folder]);
    expect(detect(prompt)).toBe(filePath);
    const safePrompt = approvedLocalFilePrompt(prompt, filePath);
    expect(safePrompt).toContain("[approved file]");
    expect(safePrompt).toContain("[approved folder]");
    expect(safePrompt).not.toContain("Lab_Inventory.csv");
    expect(safePrompt).not.toContain("/Users/example");
  });

  it("resolves every explicitly named file for a read-only strategic comparison", () => {
    const folder = "/Users/example/Documents/OOMU/Projects/mock_data";
    const prompt = `Perform a comprehensive strategic evaluation of the supplier proposals in supplier_proposals.json and cross-reference them with the requirements in q3_strategic_vendor_proposals.txt located in "${folder}". Compare technical compliance, unit pricing, and delivery risks, and provide a multi-scenario vendor trade-off matrix.`;
    const paths = directLocalFileReadPaths(prompt, candidateLocalPathsFromText(prompt));

    expect(paths).toEqual([
      `${folder}/supplier_proposals.json`,
      `${folder}/q3_strategic_vendor_proposals.txt`,
    ]);
    const safePrompt = approvedLocalFilesPrompt(prompt, paths ?? []);
    expect(safePrompt).not.toContain("/Users/example");
    expect(safePrompt).not.toContain("supplier_proposals.json");
    expect(safePrompt).not.toContain("q3_strategic_vendor_proposals.txt");
    expect(safePrompt.match(/\[approved file\]/g)).toHaveLength(2);
  });

  it("rejects a named-file batch above the attachment limit", () => {
    const folder = "/Users/example/Documents/OOMU/Projects/mock_data";
    const prompt = `Compare one.json, two.json, three.json, four.json, five.json, and six.json located in "${folder}".`;

    expect(directLocalFileReadPaths(prompt, candidateLocalPathsFromText(prompt))).toBeNull();
  });

  it("rejects more explicit paths than can be attached without truncating the request", () => {
    const prompt = ["one", "two", "three", "four", "five", "six"]
      .map((name) => `/Users/example/Documents/${name}.json`)
      .join(" and ");

    expect(candidateLocalPathsFromText(prompt)).toHaveLength(5);
    expect(directLocalFileReadPaths(prompt, candidateLocalPathsFromText(prompt))).toBeNull();
  });
});
