# 生产运维手册

## 必需配置

生产环境设置 `IAMRUST_ENV=production`，并提供 PostgreSQL、S3、TLS SMTP、至少 32 字节 JWT 密钥。管理 API 仅在配置 `IAMRUST_ADMIN_TOKEN` 后启用；上传恶意文件扫描通过 `IAMRUST_CLAMAV_ADDR` 接入。翻译和转写保持可选。

客户端远程 API 必须使用 HTTPS/WSS。TURN 凭据通过 `VITE_ICE_SERVERS` 注入，不得提交仓库。

## 健康与指标

- `/health/live`：进程存活。
- `/health/ready`：数据库连接和应用状态可用。
- `/metrics`：请求量、4xx/5xx、请求延迟、认证失败、消息成功/失败、ACK 延迟、同步积压、WebSocket 总量/活跃量和隔离文件计数。

建议告警：5 分钟 5xx 比例 > 2%；就绪失败持续 2 分钟；消息失败率 > 1%；ACK P95 超过 2 秒；同步积压持续增长；对象存储/ClamAV 日志连续失败。日志使用 JSON，`x-request-id` 用于跨代理关联，不记录密码、令牌或完整消息正文。

## 备份与恢复

每日执行 `scripts/backup-postgres.sh`，将加密后的 dump 和 SHA-256 校验文件复制到独立区域；保留 30 个日备份、12 个周备份。对象存储启用版本控制、未完成分片清理和跨区域复制，生命周期不得早于 `docs/DATA_RETENTION.md`。

恢复时创建隔离数据库，设置 `IAMRUST_RESTORE_DATABASE_URL` 后运行 `scripts/restore-postgres.sh <dump>`，验证迁移版本、关键表数量、登录和消息同步，再切换流量。CI 使用 `scripts/test-migrations.sh` 对空库、上一迁移升级和备份恢复执行演练。

## 故障操作

- 断网：客户端指数退避；恢复后先拉取游标缺口。
- PostgreSQL 重启：就绪探针摘除实例；连接恢复后服务重新加入。
- 对象存储故障：文本消息仍可用，媒体 API 返回可重试错误。
- 磁盘将满：停止接收新上传，保留消息/审计写入空间并扩容。
- 滚动更新：先部署兼容服务端，再发布客户端；至少支持当前和前一协议版本。

## 管理工具

```bash
IAMRUST_ADMIN_URL=https://service.example \
IAMRUST_ADMIN_TOKEN='32-byte-or-longer-secret' \
cargo run -p iamrust-server --bin iamrust-admin -- audit 100
```

支持 `suspend <uuid>`、`restore <uuid>`、`revoke-sessions <uuid>` 和 `audit [limit]`。同一能力也可在服务端 `/admin` 页面使用；令牌只保留在页面内存。
