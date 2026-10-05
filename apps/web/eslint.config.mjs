import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

export default defineConfig([
  ...nextVitals,
  ...nextTs,
  globalIgnores([".next/**", "out/**", "build/**", "next-env.d.ts", "src/generated/**"]),
  {
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      // Model output and learner text are rendered as text, never as HTML.
      "react/no-danger": "error",
      // Data comes only from the typed client in src/api. No other origin is contacted.
      "no-restricted-globals": [
        "error",
        { name: "fetch", message: "Use the typed client in src/api." },
        { name: "XMLHttpRequest", message: "Use the typed client in src/api." },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "NewExpression[callee.name='WebSocket']",
          message: "Open the event stream through src/api only.",
        },
      ],
    },
  },
  {
    files: ["src/api/**"],
    rules: { "no-restricted-globals": "off", "no-restricted-syntax": "off" },
  },
]);
