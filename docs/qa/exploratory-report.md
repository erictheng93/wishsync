# WishSync 探索式 QA 報告（Playwright 驅動）

日期：2026-10-09　測試者：Claude（自動化輔助手動探索）　範圍：只找問題、不改程式碼

## (a) 摘要

**環境**（全部為本機自建、獨立資源）：資料庫 `wishsync_qa`（docker db 容器）、API `:8082`（debug build，`APP_ENV=dev`、`APP_URL=http://localhost:3014`）、Web `:3014`（production build + node-server preset，輸出 `apps/web/qa/.app/.output`）。PWA 另以「與正式部署相同的 cloudflare_pages preset」建置（`apps/web/qa/.app-cf/dist`）並用 `wrangler pages dev` 在 `:3015` 驗證（原因見 F-22）。Mailpit `:8025`。腳本在 `apps/web/qa/`，證據在 `apps/web/qa/out/`（截圖 `shots/`、`*.jsonl` 原始記錄；已加入根 `.gitignore`）。

**做了什麼**：6 個範圍皆實際執行。
1. 全站爬行：匿名／訪客／創建者／staff／非 staff 進 /admin，共 52 個頁面狀態（含認領 sheet、檢舉 sheet、品項 sheet、刪除/封存/刪帳號確認、分享 sheet、後台 4 分頁與下架 sheet、條款/隱私/離線/404/410）× 4 視窗 × 亮暗 = 400 次稽核（溢位、截斷、觸控目標、alt、label、焦點、對比、console/網路）。axe-core 未安裝，對比用 `getComputedStyle` 自算。
2. 輸入探索：約 360 組 API 邊界/惡意輸入、XSS 顯示處渲染（分享頁、儀表板、編輯、進度、我的認領、後台三分頁、email）、長字串版面、貼上換行、雙擊、斷網。
3. 狀態流程：上一頁/重整/Esc/焦點、登入重導、多分頁登出入、session 過期、清單結束/封存/下架/品項刪除時開著的頁面、兩分頁衝突、驚喜模式前後、認領逾期（等背景 job 實測）、清 storage/恢復連結/失效 token、LINE/Safari UA、無 JS、storage 被禁。
4. PWA：manifest、圖示尺寸、SW 註冊/scope、快取內容、關閉 web server 的離線導覽、更新流程、標頭。
5. 安全煙霧：IDOR、未登入、CORS、cookie 屬性、Idempotency、錯誤洩漏、帳號列舉、限流。
6. 資料一致性：大量操作（含 40 路併發認領、強制刪除、帳號刪除、後台操作）後 SQL 檢查不變量。

**結果**：核心防超賣與授權設計穩固（40 併發搶 5 件 → 5 成功/35 衝突；所有 IDOR 皆 403/404；XSS 全部被轉義；不變量全過）。發現 **1 高 / 6 中 / 14 低 / 6 建議**，另有數項已確認為 by design。

## (b) 發現清單

> 標記：【已重現】= 腳本實際重現；【疑似】= 由程式碼/間接證據推得、未直接重現。

### 高

**F-01【已重現】回應遺失後重送，認領變成「孤兒」，且成功畫面謊稱已存於此裝置**
- 重現：匿名訪客送出首次認領；伺服器已處理但回應在途中遺失（`apps/web/qa/02e-lostresp.mjs`：以獨立 cookie jar 送出請求後 `route.abort`）；使用者按重試（同一 Idempotency-Key）。
- 實際：伺服器回 201 + `Idempotency-Replayed: true`，但 `claims.rs` 刻意從儲存的回應移除 `guest_token`，也無 Set-Cookie。前端 `persisted` 預設 true，顯示「✓ 認領成功！這份認領已存在此裝置，可在『我的認領』查看」；實際 `localStorage`/cookie 皆無 token（輸出：`cookies [] ls null`）。重新整理後看不到自己的認領、無法修改/取消；該認領占用庫存直到逾期（清單未設 TTL 則永久）；使用者若再認領一次，會建立第二位 guest 重複占量。
- 預期：重播也要讓使用者拿到身分，或前端偵測「201 但無 token 且本機無 token」並提示/引導（Email 恢復）。
- 證據：`out/02d.jsonl`、`out/shots/net__lostresp2.png`（成功畫面）、`02e` 終端輸出。
- 影響：所有首次認領的訪客在行動網路不穩時（本產品主要情境：LINE/手機）；資料不遺失但使用者失去控制權。
- 建議：重播時對「scope=anon」的 key 重新簽發同一 guest 的新 token（或把 token 以可解密形式短期保存）；至少前端在 `r.guest_token` 缺且 `getGuestToken()` 為空時顯示警告而非成功橫幅。

### 中

**F-02【已重現】認領 sheet 與成功 overlay 不是真正的 modal：無 Esc、無焦點陷阱、無初始焦點**
- 重現：開 `/s/{slug}` → 「我要送」→ 按 Esc；Tab 14 次。
- 實際：Esc 不關閉；初始焦點仍在被遮住的觸發按鈕；14 次 Tab 有 7 次跑出 dialog（到背景頁）；成功 overlay 開啟時焦點在 BODY、6 次 Tab 有 4 次在 dialog 外、Esc 不關閉。點背景會關閉並丟掉已輸入內容（無確認）。創建者端 `<dialog>` 版 Sheet 沒此問題。
- 預期：`role=dialog aria-modal` 必須自管焦點（或改用 `<dialog>`）；Esc 關閉。
- 證據：`out/03.jsonl`（S1.focus-trap、S1.escape-closes、S1.success-overlay-a11y、S1.backdrop-click）。
- 影響：鍵盤/螢幕閱讀器使用者無法正常使用認領（核心流程）。建議：改用與創建者端相同的 `<dialog>` 元件。

**F-03【已重現】文字欄位含 NUL（U+0000）→ HTTP 500**
- 重現：`POST /wishlists {"title":"a\u0000b"}`；同樣發生於品項 title/description/brand/spec、認領 display_name/note/contact；`GET /public/wishlists/abc%00defghi` 亦 500。
- 實際：500 `INTERNAL_ERROR`，API 日誌 `invalid byte sequence for encoding "UTF8": 0x00`（每次都記 ERROR）。預期 422（或剝除）。孤立代理字元（`\ud800`）則回 400 純文字（見 F-12）。
- 證據：`out/02a.jsonl`（`*:nul`、`path.nul`）、`out/api.log`。
- 影響：匿名端點（認領）即可製造 500 與錯誤日誌噪音；不致資料毀損。建議：在 `text()`/認領驗證統一拒絕 `\0`，路徑參數在 slug 驗證階段拒絕。

**F-04【已重現】長且無空白的字串使版面水平溢位**
- 重現：清單標題 100 字元不含空白（如 `WWWW…`）、描述 300 字元連續字、品項名稱/品牌/規格長字串、暱稱 30 字。
- 實際：分享頁 h1／描述／品項標題超出視窗（320 與 1440 皆溢位）；`/dashboard` 卡片標題、`/lists/:id/edit`、`/lists/:id/progress` 溢位（320/390，種子資料的長英數品項名稱即觸發）。爬行中 `edit-active`/`progress-active` 在 ≤390 全數溢位。
- 證據：`out/shots/in__long-share-320.png`、`in__long-dashboard-320.png`、`creator__edit-active__320__light.png`、`out/02b.jsonl`（long-*）、`out/crawl.jsonl`。
- 影響：使用者貼商品網址當品項名稱是常見行為。建議：`overflow-wrap:anywhere`（或 `word-break:break-word`）套在 h1、卡片標題、描述、暱稱等。

**F-05【已重現】失效的 guest token 讓分享頁認領永遠失敗，且只顯示「發生錯誤，請稍後再試」**
- 重現：訪客 A 的 token 因「刪除我的暱稱」或「用 Email 恢復連結換發」而失效（`/guest/recover` 換發會使舊裝置 token 失效），但舊裝置 localStorage 仍留著；到 `/s/{slug}` 按「我要送」→「確認認領」。
- 實際：sheet 仍可開啟並送出，API 回 401，前端顯示「發生錯誤，請稍後再試」；token 不會被清除，每次都失敗，只有進 `/me/claims`（遇 401 才清）才會恢復。
- 根因：`AppError::Unauthorized/NotFound/WishlistRemoved` 的 JSON 沒有 `detail`，`useGuest` 只讀 `d.detail`，因此 401/404/410 一律顯示泛用訊息。同樣造成：認領時清單被下架（410）、品項已被刪除（404）都只顯示「發生錯誤，請稍後再試」（S4.hidden、S4.itemdeleted）。
- 證據：`out/03.jsonl`（S8.staleToken_*、S4.hidden、S4.itemdeleted）、`out/shots/s8__stale-token.png`。
- 建議：前端依 `code` 對應文案；401 時清除 token 並改走新訪客流程；API 錯誤補 `detail`。

**F-06【已重現】暗色模式下紅色連結/按鈕文字對比 4.16:1（< 4.5:1）**
- 實際：暗色 `--red:#e5334f` 在卡片底 `#181818` 上為 4.16:1，影響「商品連結」「已認領 n/m」數字、「取消」「進度」「編輯」「儲存設定」「＋ 新增品項」「設定」「個資蒐集告知」等十餘種元素（分享頁、編輯頁、儀表板全部）。亮色模式主要元素通過；Nuxt 預設 404 頁暗色灰字 4.18:1（見 F-16）。
- 證據：`out/crawl.jsonl`（`lowContrast`，所有 `*/dark` 條目）。建議：暗色把文字用紅提亮到 ≥ #f0566e（約 5:1）或把卡片底改為更深。

**F-07【已重現】品項編輯沒有樂觀並行控制：兩分頁同時編輯，後存者靜默覆蓋**
- 重現：同一帳號兩分頁開 `/lists/:id/edit`，各自開同一品項、分別改名稱後儲存。
- 實際：分頁二儲存成功、無任何提示，DB 為「品項-分頁二」，分頁一的修改消失。清單層級（標題等）則正確回「這份清單已在其他地方被修改，請重新載入」。
- 證據：`out/03.jsonl`（S5.list-conflict、S5.item-conflict）。API 已支援 `expected_updated_at`（鎖定期間除外），前端 ItemSheet 未帶。建議：ItemSheet 帶 `expected_updated_at` 並處理 `STALE_VERSION`。

### 低

- **F-08【已重現】帳號刪除後，該使用者在他人清單的認領永久保持 reserved**：`DELETE /me` 只把 claimer_name 改為「已刪除的使用者」；清單若無 TTL，配額永久被占，且只有清單擁有者能取消（`out/06.jsonl` delete-account）。建議：刪帳號時取消其 reserved 認領並回補。
- **F-09【已重現】強制刪除有認領的品項：被取消的認領沒有 audit、沒有通知訪客，且已刪品項的 `qty_claimed` 殘留**（取消 2 件但 qty_claimed 仍為 2；SQL 不變量僅在「含已刪品項」時不成立，存活品項皆一致）。`out/06.jsonl` force-delete。
- **F-10【已重現】暱稱欄位 `maxlength=40`，API 限制 30**：輸入 31–40 字得到 422，且錯誤訊息「暱稱需為 1–30 字」同時出現在橫幅與欄位下方兩次（`out/shots/in__nick35.png`）。同類不一致：清單名稱欄 UI 40 / API 100、品項名稱 UI 80 / API 120（較嚴格，無害，但既有 41–100 字的清單標題在編輯頁被截成單行 h1 且表單 maxlength 與資料不符）。
- **F-11【已重現】Email 只驗證含 `@`**：`a b@c.com`、`<script>@x.com`、含 CRLF 者都回 201；寄信時被擋（notifications `failed: bad address`），但成功畫面仍顯示「已寄出管理連結到 s***@…」，訪客誤以為有備援（`out/02c.jsonl` 與 notifications 表：4 筆 failed）。合法 IDN（`用戶@例え.jp`）也被拒。
- **F-12【已重現】框架層錯誤為純文字且洩漏內部型別名**：415/413 為 `text/plain`；非法 UUID 路徑回 `Invalid URL: Cannot parse id with value …`；JSON 型別錯誤回 `Failed to deserialize the JSON body into the target type: … expected struct LoginReq at line 1 column 3`（洩漏 struct 名）；孤立代理字元 400 純文字。不含堆疊/SQL，但與 RFC 9457 慣例不一致（`out/02a.jsonl`、`out/05.jsonl` error-leak）。
- **F-13【已重現】Nuxt 預設 404 頁**：英文「Page not found」、無主題與導覽（`shots/anon__404__390__light.png`）；`/admin` 對非 staff 顯示同頁並在 console 噴 `[NUXT_E1005]` 與 h3 statusMessage 警告。
- **F-14【已重現】觸控目標 < 44px**：checkbox/radio `input` 13×13（116 次，標籤整列可點，實際影響較小）、`<summary>清單設定` 243×24、`/me/claims` 的清單連結 121×20、`/offline`/404 頁連結 100×20。
- **F-15【已重現】每個匿名訪客載入頁面都會製造 2 個 401 console error**（`/me`、`/guest/me` 探測）；e2e 已列為預期，但會淹沒真實錯誤、也可能觸發 Sentry 類工具。400 次爬行共 192 筆（`out/crawl-issues.jsonl`）。
- **F-16【已重現】控制/雙向字元與「看不見」的標題被接受**：BEL、ESC、U+202E（RTL override）、僅含零寬空白的標題/暱稱皆回 201；零寬字元標題在清單中顯示為空白。顯示處皆被 Vue 轉義，無 XSS；風險為視覺偽裝。
- **F-17【已重現】日期邊界**：非驚喜清單接受 `0000-01-01` 與 `-0001-01-01`、`9999-12-31`；驚喜模式則正確拒絕（`out/02a.jsonl` event_date）。
- **F-18【已重現】API 回應沒有 `X-Content-Type-Options`、`Cache-Control`（預設）等安全標頭**（`out/05.jsonl` security-headers-api 為空）。JSON API 風險低，建議於 Cloudflare 或中介層補 `nosniff`。
- **F-19【已重現】無 JS 時「我要送」按鈕可點但無作用**，也沒有 `<noscript>` 說明（內容/OG/進度 SSR 完整，`shots/ssr__nojs-share.png`）。`/login`、`/dashboard` 無 JS 為空白（設計為 CSR）。
- **F-20【已重現】逾期認領釋放有最長 5 分鐘延遲**：逾期未釋放期間配額仍被占；逾期但尚未釋放時，原訪客仍可把狀態改為 purchased 而「救回」（`expires_at` 被清空）。背景 job 實測 175 秒內釋放並寫入 `claim.expire` audit，功能正確（`out/03b.jsonl`）。
- **F-21【已重現】條款/隱私頁顯示「版本：草案（正式條款上線前請由法務確認）」**，上線前需處理（`out/03c.jsonl` nojs/terms）。

### 建議

- **F-22【環境差異】node-server preset 下 `/offline` 為 404，SW 無法安裝**：`sw.js` 預先快取 `/offline`，但只有 `public/offline.html`；Cloudflare Pages 會以 `/offline` 提供（`_routes.json` 已排除），正式環境正常，但 e2e/本 QA 用的 node-server 站台 SW 安裝失敗（`getRegistration()` 為 null，快取空）。因此現有 E2E 無法驗證 PWA。建議：加 `pages/offline.vue` 或 nitro `publicAssets`/routeRules 讓 `/offline` 在兩種 preset 皆 200，並補 SW 的 E2E。
- **F-23**：SW 的 `/_nuxt/*` 快取只在手動改 `CACHE_VERSION` 時清除，每次部署的雜湊資產持續累積（實測 38 筆）；建議以 build id 自動換版或限制數量。
- **F-24**：登入以 email 計 5 次失敗即鎖 5 分鐘（含正確密碼），任何人可持續鎖死他人登入（已重現，`out/05.jsonl` ratelimit）。屬常見取捨（by design 傾向），可考慮「email+IP」計數。
- **F-25**：確認信可被指定寄給任意第三方 Email（未驗證，每 email 3 封/小時、每 IP 100 新訪客/小時），可被用來騷擾；信內容為中性但含管理連結。
- **F-26**：`GET /unsubscribe?token=` 為狀態變更的 GET，信件掃描器預取可能誤退訂；建議點擊後頁面再 POST 確認。
- **F-27**：每 IP 每小時建立 100 位新訪客：大型活動共用 NAT 可能誤擋（程式註解已知取捨）。

### 已確認為 by design（不計入數量）
- 驚喜鎖定期間，公開 API 仍回傳 `qty_claimed`、整體完成度；僅前端軟性遮蔽（程式註解與頁面說明已揭露）。單品項清單的完成度百分比會洩漏有人認領。
- 匿名重播：不同匿名者以相同 Idempotency-Key（UUID，不可猜）與相同內容可取得同一認領回應（無 token）。
- 逾期前 purchased 可救回（FR-08 的語意）。
- `/admin` 對非 staff 顯示 404（隱藏存在）。
- 重複 CORS：非 APP_URL 的 Origin 預檢仍回 200，但 `Access-Control-Allow-Origin` 固定為 APP_URL，瀏覽器會擋（正確）。

## (c) 通過的檢查清單

- **授權/IDOR**：A 對 B 的清單 GET/PATCH/DELETE/dashboard/新增品項/改品項/刪品項/reorder 全為 404；非擁有者的登入者改他人認領 403；A 的 guest token 改/刪 B 的認領 403；匿名改認領 401；非 staff 打 /admin/* 全 403；未登入打創建者 API 全 401；`/me/export` 不含他人資料。
- **防超賣**：40 路併發搶 5 件 → 5×201、35×409，DB `qty_claimed=5`；同一認領 6 路併發改量 → 全 200、`qty_claimed` 與認領量一致。
- **冪等**：同 key 不同內容 → 409；`idempotency_keys` 內無 `guest_token` 明文（0 筆，亦不在 scope 欄）；`guests.guest_token_hash` 皆為 32 bytes 雜湊。
- **XSS**：`<img onerror>`、`"><script>`、`<svg onload>`、`javascript:` 連結、模板語法（`{{7*7}}`）放在清單標題/說明、品項各欄、暱稱、備註、聯絡方式、檢舉內容、創建者暱稱；於分享頁（匿名/訪客/擁有者）、儀表板、編輯、進度、設定、我的認領、後台三分頁皆被轉義，無 dialog、無 `window.__xss`、無注入節點。`product_url` 僅接受 http(s)（`javascript:`/`data:`/`file:`/`ftp:`/含 tab 皆 422）。SSR `<title>`、`og:*`、`description` 對 `<b>"&` 正確處理。
- **Email**：純文字信件，無 HTML 注入；標題含 CRLF + `Bcc:` 不會造成標頭注入（Mailpit 無 Bcc/自訂標頭）；通知內容不含品項/認領者。
- **帳號列舉**：註冊已存在/不存在、重設密碼已知/未知 email 回應一致；登入錯誤訊息一致。
- **CORS / cookie**：僅 APP_URL 被回傳於 ACAO；`ws_session`：HttpOnly、SameSite=Lax、Max-Age 30 天，dev 無 Secure（`APP_ENV=dev` 才省略，合理）；`ws_guest`：HttpOnly、Secure、SameSite=Lax。認領回應 `Cache-Control: private, no-store`。
- **限流**：登入 5 次失敗後 429（含 Retry-After 300）；檢舉每清單每 IP 1 次/日、每 IP 10 次/時；OTP 請求 429；同 guest 同清單認領 15 次/時；匯出 3 次/日；超限不佔用 Idempotency key。
- **請求強健性**：非法 JSON/陣列/空 body 400、錯誤 Content-Type 415、2MB/20MB body 413、深層巢狀 JSON 400（未崩潰）、70KB header/100KB cookie 不崩、limit=0/-1/超大 422/400、cursor 亂碼 422、qty 0/負/小數/字串/null/100/2^31 皆 422、`TRACE`/`PUT /me` 405。
- **錯誤回應**：未見堆疊、SQL、檔案路徑；500 僅「系統錯誤」。
- **狀態流程**：結束/封存清單時開著的訪客頁認領得到明確訊息（「此清單已結束」）；重新整理後分別顯示「已結束」橫幅/「找不到這份清單」；下架顯示 410 頁；創建者在其他頁遭下架/結束/封存後存檔，顯示下架原因或「清單已關閉或封存」；清單層級兩分頁衝突有 STALE 提示；session 過期後儲存會導向 `/login?redirect=…`，重新登入回原頁；一分頁登出後另一分頁操作被導向登入；登入後 redirect 對 `https://evil…`、`//evil…`、`javascript:` 皆落在 `/dashboard`（無 open redirect）；登入後上一頁不回登入表單。
- **驚喜模式**：鎖定期間 dashboard `claims=null`、品項 `qty_claimed=null`、公開頁無 claimers、`/me/export` 無認領者；擁有者刪除/降量/取消他人認領/關閉驚喜皆 403；event_date 設為昨天即解鎖並顯示認領者與備註；event_date＝今日（台北）已解鎖、明日仍鎖定。
- **訪客恢復**：清 storage/cookie 後顯示為未認領（可再認領，建立新 guest——符合無帳號設計）；Email 確認信的 `#r=` 連結可恢復，網址立即清除 fragment；LINE UA 顯示提示橫幅；storage 全被禁時成功頁正確警告「無法儲存於此裝置」；斷線時 sheet 按鈕顯示「離線中，暫時無法認領」並於恢復後可用；請求在送出前失敗時同 key 重送，只建立 1 筆認領；雙擊「確認認領」只送 1 次請求。
- **SSR/SEO**：LINE/Safari/FB/curl UA 的 SSR HTML 完全相同且完整（h1、品項、og:title/description/image/url、twitter:card）；404/草稿 → 404+noindex；下架 → 410+noindex 且不洩漏標題；無 JS 仍可閱讀內容與認領者名稱。
- **PWA（cloudflare_pages preset，`wrangler pages dev`）**：manifest 200、`application/manifest+json`、欄位齊全（start_url `/dashboard?source=pwa`、scope `/`、standalone）；3 個圖示 200 且實際像素與宣告一致（192/512/512 maskable）；SW 註冊 scope `/`、reload 後受控；快取只有 `/offline`、圖示與 `/_nuxt/*` 雜湊資產（38 筆；無 `/s/*`、無 `/api`、無 HTML 導覽、無 `/_nuxt/builds/*`）；改標題後立即重整仍是最新（導覽 network-only）；另一使用者無法看到前一使用者的 `/dashboard`；關閉 web server 後，`/s/*`、`/dashboard`、`/terms` 導覽皆回離線頁、已快取資產仍可取得；改 `CACHE_VERSION` 後：新 SW 進入 waiting（不 skipWaiting），所有分頁關閉後啟用且舊快取 `v1` 被刪除；`sw.js` 標頭 `Cache-Control: no-cache`、manifest `max-age=86400`、圖示 `max-age=604800`。
- **資料一致性（SQL）**：存活品項 `qty_claimed` = 有效認領量總和（mismatch=0）；無 `qty_claimed>qty_needed` 或負值；無孤兒 claims/items；無重複有效認領；每筆認領皆有 `claim.create` audit，取消/逾期/擁有者操作皆有 audit（`claim.update`、`claim.owner_update`、`claim.expire`）；後台檢舉處理、下架/恢復、旗標變更、匯出、訪客刪除皆有 audit；`read_only` 旗標開啟時認領回 503、關閉後恢復。
- **版面/無障礙**：400 次爬行中，除 F-04 外無水平溢位；所有 `<img>` 皆有 alt（品項圖為 `alt=""` 裝飾性）；表單控制項皆有 label（唯一例外：分享 sheet 的唯讀連結輸入框 `#c-share-input` 無 label/aria-label）；Tab 走訪的焦點皆有可見樣式（紅色 2px outline），僅 `/lists/new` 的一個 input 未偵測到。

## (d) 未涵蓋與限制

- 非真實 CDN/Cloudflare 環境：HTTP 標頭（HSTS、CSP 等）、`TRUSTED_PROXY`、Turnstile、S3/R2 圖片上傳（MinIO 流程、圖片 pending/ready）、OAuth（Google/LINE）皆未測。
- 只用 Chromium；未測 WebKit/Firefox 的實際 Safari 行為（僅以 UA 字串模擬）。LINE 內建瀏覽器的 storage 清除行為以腳本模擬。
- 螢幕閱讀器、色盲、縮放 200%、`prefers-reduced-motion` 未人工驗證。
- SSE 即時更新只觀察到「即時更新中」狀態；未做長時間斷線/輪詢降級實測。
- 逾期通知信、活動日提醒信（`event.reminder`）、每日摘要（09:00 台北）未等待實測。
- 環境注意：測試期間主機曾睡眠/斷網約數小時，導致一次背景 job 與 API 連線池逾時（API 日誌 `pool timed out`），已重跑受影響的測項；信中退訂連結為 `localhost:8080`，因本次未設 `API_BASE_URL`，屬環境設定非缺陷。
- 「提示頻率限制」計數在測試間以 `DELETE FROM rate_limits` 重置（僅 `wishsync_qa`）。
- 對比檢查為自算（未用 axe）：未涵蓋漸層/圖片上的文字，可能漏報或少量誤報。

## (e) 建議後續自動化（值得補成 E2E / 整合測試）

1. **F-01 回應遺失重送**（E2E：`route` 用獨立 jar 送出後 abort；斷言重送後 `localStorage` 有 token，或畫面顯示警告）。這是最有價值的一條。
2. **F-02 認領 sheet 鍵盤行為**：Esc 關閉、Tab 不離開、初始焦點在 dialog 內（也可用 axe-core 掃 `role=dialog`）。
3. **F-03 NUL 字元**：API 整合測試 `title='a\u0000b'` 應 422。
4. **F-04 長字串溢位**：E2E 建 100 字無空白標題，斷言 `scrollWidth <= innerWidth`（320 視窗，分享頁/儀表板/編輯/進度）。
5. **F-05/F-06/錯誤文案**：失效 token 後認領的訊息與 token 清除；API 對 401/404/410 補 `detail` 的契約測試。
6. **F-07 品項衝突**：兩 context 編輯同品項，斷言第二位收到 STALE 提示。
7. **PWA**：在與正式相同的 preset（或讓 `/offline` 在 node-server 也 200）下補 E2E：SW 註冊、快取內容白名單（不含 `/s/*`、`/api`）、關閉 server 後導覽回離線頁。
8. **不變量**：把 06 的 SQL 檢查做成每次 E2E 結束後的 `afterAll`（含強制刪除、帳號刪除情境）。
9. **對比/觸控目標**：把 `AUDIT_FN`（`apps/web/qa/lib.mjs`）縮成煙霧測試，或引入 axe-core 對 8 個主要頁面 × 亮暗色掃描。

## 附：腳本與證據索引

`apps/web/qa/`：`lib.mjs`（共用）、`seed.mjs`、`01-crawl.mjs`、`02a-api-inputs.mjs`、`02b-ui-inputs.mjs`、`02c-mail.mjs`、`02d-network.mjs`、`02e-lostresp.mjs`、`03-state.mjs`、`03b-expiry.mjs`、`03c-ssr.mjs`、`04-pwa.mjs`、`05-security.mjs`、`06-invariants.mjs`、`06b-delete.mjs`；建置 layer：`.app/`（node-server）、`.app-cf/`（cloudflare_pages，僅供 PWA 驗證）。輸出在 `apps/web/qa/out/`（`crawl.jsonl`、`crawl-issues.jsonl`、`02a/02b/02c/02d/03/03b/03c/04/05/06.jsonl`、`shots/*.png`、`api.log`）。

## (f) 修復狀態（2026-10-09）

| 編號 | 狀態 | 說明 |
|---|---|---|
| F-01 | 已修 | 重播時對同一 guest 重新簽發 token（DB 不存明文，舊 token 失效）；前端防呆警告；測試 `lost_first_response_replay_reissues_usable_token` |
| F-02 | 已修 | 認領 / 成功 / 檢舉改用原生 `<dialog>`（Esc、焦點陷阱、初始焦點、焦點歸還）；E2E `guest-modal.spec.ts` |
| F-03、F-16 | 已修 | 集中式文字驗證（NUL、控制字元、雙向覆寫、僅零寬字元）；`%00` 路徑回 404 |
| F-04 | 已修 | `overflow-wrap:anywhere`；E2E `guest-longtext.spec.ts` |
| F-05 | 已修 | 依錯誤碼對應文案；401 自動清 token 並以新訪客重試；後端對「只有 cookie 帶的失效 token」視為匿名（`tests/stale_cookie.rs`） |
| F-06 | 已修 | 暗色文字用紅 `--red-text`（≥ 4.5:1），訪客端與創建者端皆套用 |
| F-07 | 已修 | 品項編輯帶 `expected_updated_at`，衝突提示重新載入 |
| F-08 | 已修 | 帳號刪除時取消其保留中的認領並回補數量 |
| F-09 | 已修 | 強制刪除品項：審計、數量歸零、通知受影響者（含專屬信件文案） |
| F-10 | 已修 | 前端 maxlength 與 API 一致；錯誤不重複顯示 |
| F-11 | 已修 | Email 以可寄送格式驗證（合法 IDN 仍被拒，函式庫限制） |
| F-12 | 已修 | 框架錯誤統一為 problem+json，不洩漏型別名 |
| F-13 | 已修 | 新增主題化 `error.vue` |
| F-14 | 已修 | 觸控目標 ≥ 44px（summary、連結） |
| F-17 | 已修 | 活動日限制 2000–2100 |
| F-18 | 已修 | `nosniff` / `no-referrer` / `no-store` |
| F-19 | 已修 | 無 JS 說明，隱藏「我要送」 |
| F-22 | 已修 | `/offline` 於兩種 preset 皆 200；E2E `pwa.spec.ts` |
| F-24 | 已修 | 登入鎖定改 (email, IP) 5 次、email 總量 20 次、IP 30 次 |
| F-26 | 已修 | 退訂改兩步（GET 只轉址，POST 才退訂）；E2E `unsubscribe.spec.ts` |
| F-15、F-20、F-21、F-23、F-25、F-27 | 未修 | F-15 等有 Sentry 時一併處理；F-20 / F-25 / F-27 為設計取捨；F-21 待法務；F-23 影響小 |
