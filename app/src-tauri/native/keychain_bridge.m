#import <Foundation/Foundation.h>
#import <Security/Security.h>
#include "keychain_bridge.h"
#include <string.h>

static NSMutableDictionary *echo_keychain_query(NSString *service, NSString *account) {
    return [@{
        (__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
        (__bridge id)kSecAttrService: service,
        (__bridge id)kSecAttrAccount: account,
    } mutableCopy];
}

int echo_keychain_set(const char *service, const char *account, const char *password) {
    if (service == NULL || account == NULL || password == NULL) {
        return 1;
    }
    NSString *service_string = [NSString stringWithUTF8String:service];
    NSString *account_string = [NSString stringWithUTF8String:account];
    if (service_string == nil || account_string == nil) {
        return 1;
    }
    NSData *password_data = [NSData dataWithBytes:password length:strlen(password)];
    if (password_data == nil) {
        return 1;
    }

    NSMutableDictionary *query = echo_keychain_query(service_string, account_string);
    OSStatus status = SecItemCopyMatching((__bridge CFDictionaryRef)query, NULL);
    if (status == errSecSuccess) {
        NSDictionary *update = @{(__bridge id)kSecValueData: password_data};
        status = SecItemUpdate((__bridge CFDictionaryRef)query, (__bridge CFDictionaryRef)update);
        if (status != errSecSuccess) {
            return 3;
        }
    } else if (status == errSecItemNotFound) {
        NSMutableDictionary *item = echo_keychain_query(service_string, account_string);
        item[(__bridge id)kSecValueData] = password_data;
        status = SecItemAdd((__bridge CFDictionaryRef)item, NULL);
        if (status != errSecSuccess) {
            return 4;
        }
    } else {
        return 2;
    }

    // 写后读回校验：CLI 的 stdin 写入曾静默写空密码（exit 0），
    // 这里必须确认钥匙串里真的是这段数据，否则调用方会把「保存成功」
    // 告诉用户，之后每次使用却报「没有凭据」。
    NSMutableDictionary *read_back = echo_keychain_query(service_string, account_string);
    read_back[(__bridge id)kSecReturnData] = @YES;
    read_back[(__bridge id)kSecMatchLimit] = (__bridge id)kSecMatchLimitOne;
    CFTypeRef found = NULL;
    status = SecItemCopyMatching((__bridge CFDictionaryRef)read_back, &found);
    if (status != errSecSuccess || found == NULL) {
        return 5;
    }
    NSData *stored = (__bridge NSData *)found;
    if (![stored isEqualToData:password_data]) {
        return 5;
    }
    return 0;
}
