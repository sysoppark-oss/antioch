// 저장소 뿌리의 파일을 읽는다 (시험은 app/ 에서 돈다)
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const readRoot = (rel: string): any => JSON.parse(readFileSync(resolve(process.cwd(), "..", rel), "utf8"));
