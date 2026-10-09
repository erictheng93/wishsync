-- 固定視窗計數器（ratelimit::check）；舊視窗由每小時清除 task 刪除
CREATE TABLE rate_limits (
  key          text        NOT NULL,
  window_start timestamptz NOT NULL,
  count        integer     NOT NULL DEFAULT 0,
  PRIMARY KEY (key, window_start)
);
CREATE INDEX rate_limits_window_idx ON rate_limits (window_start);
