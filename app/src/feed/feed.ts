// 섀시가 랙을 보는 창구. 1단계는 목업(mock.ts), 3단계에서 epistle 실클라이언트(Tauri 명령)로 바꾼다.
// 돌려주는 값은 모두 unknown 이다 — 모듈에서 온 자료이므로 섀시가 validate.ts 로 거른 뒤에만 쓴다.

export interface SlotInfo {
  /** 등록부 HKLM\SOFTWARE\Antioch\Modules\<id> 의 키 이름 */
  id: string;
  /** 파이프 이름이 공개돼 있는가(실행 중) */
  running: boolean;
  /** 공개된 파이프 이름이 바뀌었는지 알아보는 표식(값 자체는 뜻이 없다). 바뀌면 섀시가 재시도 대기를 푼다 */
  token?: string;
}

export interface ModuleLink {
  hello(): Promise<unknown>;
  describe(): Promise<unknown>;
  health(): Promise<unknown>;
  widgets(ids: string[]): Promise<unknown>;
  /** rack.subscribe. 알림 rack.event 의 params 가 onEvent 로, 연결이 끊기면 onClose 가 한 번 불린다 */
  subscribe(onEvent: (params: unknown) => void, onClose: () => void): Promise<void>;
  close(): void;
}

export interface RackFeed {
  /** 등록부를 읽어 슬롯 목록을 돌려준다 (5.2의 1번) */
  discover(): Promise<SlotInfo[]>;
  /** 슬롯에 붙는다. 서버 확인(5.2의 3번)은 이 안에서 끝난다 — 실패하면 던진다 */
  connect(id: string): Promise<ModuleLink>;
}
