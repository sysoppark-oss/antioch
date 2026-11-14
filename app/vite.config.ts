import { defineConfig } from "vitest/config";

// 말씀 자료 묶음(../word)과 계약 예시(../fixtures)를 저장소 뿌리에서 그대로 읽는다
export default defineConfig({
  server: { fs: { allow: [".."] }, strictPort: true, port: 1420 },
  build: { target: "es2022" },
  test: { environment: "jsdom", include: ["tests/**/*.test.ts"] },
});
