/* Test-only PAM module: proves the KDE lock screen displays our conversation messages and
 * unlocks on our verdict, before any daemon, phone or crypto exists.
 *
 * NOT the real module and never installed by packaging. Authorisation here is "a root-owned
 * file exists", which is safe because /run is 0755 root:root — an unprivileged attacker
 * cannot create the trigger, and an attacker who is already root owns the machine anyway.
 *
 * Build:  gcc -shared -fPIC -o pam_kpk_stub.so pam/tests/pam_kpk_stub.c -lpam
 */

#define _GNU_SOURCE
#define PAM_SM_AUTH

#include <errno.h>
#include <syslog.h>

#include <security/pam_ext.h>
#include <security/pam_modules.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>

#define DEFAULT_TRIGGER "/run/kpasskey-test-approve"
#define DEFAULT_TIMEOUT_SECONDS 20
#define POLL_INTERVAL_MS 200

struct options {
    const char *trigger;
    long timeout_seconds;
};

static struct options parse_options(int argc, const char **argv)
{
    struct options options = { DEFAULT_TRIGGER, DEFAULT_TIMEOUT_SECONDS };

    for (int i = 0; i < argc; ++i) {
        if (strncmp(argv[i], "trigger=", 8) == 0) {
            options.trigger = argv[i] + 8;
        } else if (strncmp(argv[i], "timeout=", 8) == 0) {
            char *end = NULL;
            const long value = strtol(argv[i] + 8, &end, 10);
            if (end != NULL && *end == '\0' && value > 0 && value <= 120)
                options.timeout_seconds = value;
        }
    }

    return options;
}

/* The trigger must be owned by root, so that a permissive `trigger=` in the pam.d line
   cannot turn this into a user-writable bypass. */
static int approval_present(const char *path)
{
    struct stat info;
    if (stat(path, &info) != 0)
        return 0;
    return info.st_uid == 0;
}

static long monotonic_seconds(void)
{
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0)
        return 0;
    return now.tv_sec;
}

static void sleep_poll_interval(void)
{
    struct timespec interval = { 0, POLL_INTERVAL_MS * 1000L * 1000L };
    struct timespec remaining;
    while (nanosleep(&interval, &remaining) != 0 && errno == EINTR)
        interval = remaining;
}

int pam_sm_authenticate(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)flags;

    const struct options options = parse_options(argc, argv);

    const char *service = NULL;
    if (pam_get_item(pamh, PAM_SERVICE, (const void **)&service) != PAM_SUCCESS || service == NULL)
        service = "?";

    const char *user = NULL;
    if (pam_get_user(pamh, &user, NULL) != PAM_SUCCESS || user == NULL)
        return PAM_AUTHINFO_UNAVAIL;

    pam_syslog(pamh, LOG_NOTICE, "stub: service=%s user=%s trigger=%s timeout=%lds", service, user,
               options.trigger, options.timeout_seconds);
    pam_info(pamh, "kpasskey test (%s): waiting for approval", service);

    const long deadline = monotonic_seconds() + options.timeout_seconds;
    while (monotonic_seconds() < deadline) {
        if (approval_present(options.trigger)) {
            pam_info(pamh, "Approved");
            pam_syslog(pamh, LOG_NOTICE, "stub: approved via %s", options.trigger);
            return PAM_SUCCESS;
        }
        sleep_poll_interval();
    }

    pam_error(pamh, "No approval within %lds", options.timeout_seconds);
    pam_syslog(pamh, LOG_NOTICE, "stub: no approval, returning PAM_AUTHINFO_UNAVAIL");
    return PAM_AUTHINFO_UNAVAIL;
}

int pam_sm_setcred(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)pamh;
    (void)flags;
    (void)argc;
    (void)argv;
    return PAM_SUCCESS;
}
