#ifndef echo_keychain_bridge_h
#define echo_keychain_bridge_h

// 钥匙串读写桥接（Security 框架）。
//
// 为什么不用 `security add-generic-password` CLI：2026-11-11 在 macOS 15.6
// 实测，经 stdin/文件重定向传给该 CLI 的密码会被**静默丢弃**——命令返回 0，
// 条目却存了空密码，之后每次读取都当作「没有凭据」。唯一可用的 CLI 写法
// `-w <密码>` 会把密钥暴露在进程参数（ps 可见）里。原生 SecItem* 两条都避开。
//
// 返回值：0 成功；1 参数无效；2 查询失败；3 更新失败；4 新增失败；5 写后读回校验失败。

int echo_keychain_set(const char *service, const char *account, const char *password);

#endif
