#import "tid.h"
#import <LocalAuthentication/LocalAuthentication.h>

void* create_la_context() {
    return (__bridge void*)[[LAContext alloc] init];
}

void drop_la_context(void* ctx) {
    @autoreleasepool {
        [(__bridge LAContext*)ctx release];
    }
}

void* create_keychain_la_context() {
    @autoreleasepool {
        LAContext* context = [[LAContext alloc] init];
        context.localizedReason = @"Unlock Nocterm credential vault";
        context.localizedFallbackTitle = @"";
        context.touchIDAuthenticationAllowableReuseDuration = 0;
        return (__bridge void*)context;
    }
}

void* retain_la_context(void* ctx) {
    @autoreleasepool {
        return (__bridge void*)[(__bridge LAContext*)ctx retain];
    }
}

void invalidate_la_context(void* ctx) {
    @autoreleasepool {
        [(__bridge LAContext*)ctx invalidate];
    }
}

void set_localized_cancel_title(void* ctx, char* reason) {
    NSString* titleString = [[NSString alloc] initWithCString:reason encoding:NSUTF8StringEncoding];
    [(__bridge LAContext*)ctx setLocalizedCancelTitle:titleString];
    [titleString release];
}

int32_t can_evaluate_policy(void *ctx, int32_t policy) {
    BOOL result = [(__bridge LAContext*)ctx canEvaluatePolicy:(LAPolicy)policy error:nil];
    return result;
}

void evaluate_policy(void* ctx, int32_t policy, char* reason, void* future, void* callback) {
    NSString* reasonString = [[NSString alloc] initWithCString:reason encoding:NSUTF8StringEncoding];
    [
        (__bridge LAContext*)ctx
        evaluatePolicy:(LAPolicy)policy
        localizedReason:reasonString
        reply:^(BOOL success, NSError *error) {
            if (!success && error != nil) {
                ((void (*)(void*, BOOL, int32_t))callback)(future, success, (int32_t)error.code);
            } else if (success && error == nil) {
                ((void (*)(void*, BOOL, int32_t))callback)(future, success, 0);
            }
        }
    ];
    [reasonString release];
}
