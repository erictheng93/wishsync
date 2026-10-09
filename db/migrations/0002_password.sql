-- 密碼登入：雜湊只存在 email identity；OTP 增加用途（登入 / 註冊 / 重設）；登入失敗計數供限流。
ALTER TABLE auth_identities
  ADD COLUMN password_hash text,
  ADD CONSTRAINT auth_identities_pw_email_only CHECK (password_hash IS NULL OR provider = 'email');

ALTER TABLE otp_challenges
  ADD COLUMN purpose text NOT NULL DEFAULT 'login' CHECK (purpose IN ('login','register','reset')),
  ADD COLUMN pending_password_hash text,   -- 註冊：OTP 驗證通過才寫入 auth_identities
  ADD COLUMN pending_display_name text;

CREATE TABLE login_failures (
  id          bigserial PRIMARY KEY,
  email       text        NOT NULL,
  ip          text        NOT NULL,
  created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX login_failures_email_idx ON login_failures (email, created_at DESC);
CREATE INDEX login_failures_ip_idx ON login_failures (ip, created_at DESC);
