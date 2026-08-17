/* Test-only PAM module for the walking skeleton: asks the spike daemon for a phone
 * approval and maps the verdict onto PAM return codes.
 *
 * NOT the real module. The real one talks to a root-owned daemon over the system D-Bus with
 * the hardened bus-address checks in docs/. Here the daemon runs as the desktop user while
 * this module runs as root under polkit, which inverts the trust relationship — acceptable
 * for a bench demo on one machine, never shippable.
 *
 * Build: gcc -shared -fPIC -o pam_kpk_spike.so pam/tests/pam_kpk_spike.c -lpam
 */

#define _GNU_SOURCE
#define PAM_SM_AUTH

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <syslog.h>
#include <unistd.h>

#include <security/pam_ext.h>
#include <security/pam_modules.h>

#define DEFAULT_SOCKET "/run/kpasskey/pam.sock"
#define DEFAULT_TIMEOUT_SECONDS 120
#define REPLY_MAX 1024
#define REQUEST_MAX 1024

struct options {
    const char *socket_path;
    long timeout_seconds;
};

static struct options parse_options(int argc, const char **argv)
{
    struct options options = { DEFAULT_SOCKET, DEFAULT_TIMEOUT_SECONDS };

    for (int i = 0; i < argc; ++i) {
        if (strncmp(argv[i], "socket=", 7) == 0) {
            options.socket_path = argv[i] + 7;
        } else if (strncmp(argv[i], "timeout=", 8) == 0) {
            char *end = NULL;
            const long value = strtol(argv[i] + 8, &end, 10);
            if (end != NULL && *end == '\0' && value > 0 && value <= 900)
                options.timeout_seconds = value;
        }
    }

    return options;
}

/* Only enough escaping for the fields we send; the daemon rejects malformed JSON. */
static int json_escape(const char *input, char *output, size_t size)
{
    size_t used = 0;
    for (const char *cursor = input; *cursor != '\0'; ++cursor) {
        const char c = *cursor;
        if (c == '"' || c == '\\') {
            if (used + 2 >= size)
                return -1;
            output[used++] = '\\';
            output[used++] = c;
        } else if ((unsigned char)c < 0x20) {
            continue;
        } else {
            if (used + 1 >= size)
                return -1;
            output[used++] = c;
        }
    }
    if (used >= size)
        return -1;
    output[used] = '\0';
    return 0;
}

static int connect_daemon(const char *path, long timeout_seconds)
{
    struct sockaddr_un address;
    memset(&address, 0, sizeof(address));
    address.sun_family = AF_UNIX;
    if (strlen(path) >= sizeof(address.sun_path))
        return -1;
    strncpy(address.sun_path, path, sizeof(address.sun_path) - 1);

    const int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0)
        return -1;

    struct timeval timeout = { .tv_sec = timeout_seconds, .tv_usec = 0 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));

    if (connect(fd, (const struct sockaddr *)&address, sizeof(address)) != 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static int write_all(int fd, const char *data, size_t length)
{
    size_t written = 0;
    while (written < length) {
        const ssize_t n = write(fd, data + written, length - written);
        if (n < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        written += (size_t)n;
    }
    return 0;
}

static int read_line(int fd, char *buffer, size_t size)
{
    size_t used = 0;
    while (used + 1 < size) {
        char c = 0;
        const ssize_t n = read(fd, &c, 1);
        if (n < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        if (n == 0)
            break;
        if (c == '\n')
            break;
        buffer[used++] = c;
    }
    buffer[used] = '\0';
    return (int)used;
}

int pam_sm_authenticate(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)flags;

    const struct options options = parse_options(argc, argv);

    const char *service = NULL;
    if (pam_get_item(pamh, PAM_SERVICE, (const void **)&service) != PAM_SUCCESS || service == NULL)
        service = "unknown";

    const char *user = NULL;
    if (pam_get_user(pamh, &user, NULL) != PAM_SUCCESS || user == NULL)
        return PAM_AUTHINFO_UNAVAIL;

    char safe_user[256];
    char safe_service[256];
    if (json_escape(user, safe_user, sizeof(safe_user)) != 0
        || json_escape(service, safe_service, sizeof(safe_service)) != 0)
        return PAM_AUTHINFO_UNAVAIL;

    const int fd = connect_daemon(options.socket_path, options.timeout_seconds);
    if (fd < 0) {
        pam_syslog(pamh, LOG_NOTICE, "spike: cannot reach %s: %m", options.socket_path);
        return PAM_AUTHINFO_UNAVAIL;
    }

    char request[REQUEST_MAX];
    const int length = snprintf(request, sizeof(request),
                                "{\"user\":\"%s\",\"action\":\"%s\",\"detail\":\"authorising %s\"}\n",
                                safe_user, safe_service, safe_service);
    if (length < 0 || (size_t)length >= sizeof(request)) {
        close(fd);
        return PAM_AUTHINFO_UNAVAIL;
    }

    pam_info(pamh, "Approve on your phone to continue");

    if (write_all(fd, request, (size_t)length) != 0) {
        close(fd);
        pam_syslog(pamh, LOG_NOTICE, "spike: sending the request failed: %m");
        return PAM_AUTHINFO_UNAVAIL;
    }

    char reply[REPLY_MAX];
    const int read_bytes = read_line(fd, reply, sizeof(reply));
    close(fd);

    if (read_bytes <= 0) {
        pam_syslog(pamh, LOG_NOTICE, "spike: no verdict from the daemon");
        pam_error(pamh, "Phone approval timed out");
        return PAM_AUTHINFO_UNAVAIL;
    }

    /* The daemon answers {"ok":true,...} only after verifying the signature. */
    if (strstr(reply, "\"ok\":true") != NULL) {
        pam_syslog(pamh, LOG_NOTICE, "spike: approved by phone for %s (%s)", user, service);
        return PAM_SUCCESS;
    }

    pam_syslog(pamh, LOG_NOTICE, "spike: denied: %s", reply);
    pam_error(pamh, "Phone did not approve");
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
