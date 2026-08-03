import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "sidecars/**/dist/**",
    "src-tauri/target/**",
    "tools/**/target/**",
    "venv/**",
    ".venv/**",
    "next-env.d.ts",
  ]),
  {
    files: ["src/app/components/**/*.{ts,tsx}", "src/components/**/*.{ts,tsx}"],
    ignores: [
      "src/app/components/ChatScreen.tsx",
      "src/app/components/__tests__/ChatScreen.test.tsx",
      "src/app/components/WorkflowDesigner.tsx",
    ],
    rules: {
      "max-lines": [
        "error",
        {
          max: 1500,
          skipBlankLines: true,
          skipComments: true,
        },
      ],
    },
  },
  {
    files: ["src/app/components/ChatScreen.tsx"],
    rules: {
      "max-lines": "off",
    },
  },
  {
    files: ["src/app/components/WorkflowDesigner.tsx"],
    rules: {
      "max-lines": [
        "warn",
        {
          max: 3900,
          skipBlankLines: true,
          skipComments: true,
        },
      ],
    },
  },
  {
    files: ["sidecars/whatsapp-sidecar/src/**/*.mjs"],
    rules: {
      // Baileys exposes a non-React helper with a hook-shaped upstream name.
      "react-hooks/rules-of-hooks": "off",
    },
  },
]);

export default eslintConfig;
