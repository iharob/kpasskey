#define _GNU_SOURCE

#include <pwd.h>
#include <security/pam_appl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const char *msg_style_name(int style)
{
    switch (style) {
    case PAM_PROMPT_ECHO_OFF:
        return "PAM_PROMPT_ECHO_OFF";
    case PAM_PROMPT_ECHO_ON:
        return "PAM_PROMPT_ECHO_ON";
    case PAM_ERROR_MSG:
        return "PAM_ERROR_MSG";
    case PAM_TEXT_INFO:
        return "PAM_TEXT_INFO";
    default:
        return "PAM_UNKNOWN_STYLE";
    }
}

/* Mirrors kscreenlocker's treatment of a non-interactive authenticator: never answer a
   prompt, so a module that tries to prompt is reported rather than silently satisfied. */
static int probe_conv(int num_msg, const struct pam_message **msg, struct pam_response **resp, void *appdata)
{
    (void)appdata;

    if (num_msg <= 0)
        return PAM_CONV_ERR;

    struct pam_response *replies = calloc((size_t)num_msg, sizeof(*replies));
    if (replies == NULL)
        return PAM_BUF_ERR;

    for (int i = 0; i < num_msg; ++i) {
        const int style = msg[i]->msg_style;
        printf("  conv: %-20s %s\n", msg_style_name(style), msg[i]->msg ? msg[i]->msg : "(null)");
        if (style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON) {
            printf("  conv: refusing to answer a prompt (non-interactive probe)\n");
            free(replies);
            return PAM_CONV_ERR;
        }
    }

    *resp = replies;
    return PAM_SUCCESS;
}

int main(int argc, char **argv)
{
    if (argc < 2 || argc > 3) {
        fprintf(stderr, "usage: %s <pam-service> [username]\n", argv[0]);
        return 2;
    }

    const char *service = argv[1];
    const char *user = NULL;
    if (argc == 3) {
        user = argv[2];
    } else {
        const struct passwd *pw = getpwuid(getuid());
        if (pw == NULL) {
            fprintf(stderr, "getpwuid(%u) failed\n", getuid());
            return 2;
        }
        user = pw->pw_name;
    }

    const struct pam_conv conv = { probe_conv, NULL };
    pam_handle_t *pamh = NULL;

    int rc = pam_start(service, user, &conv, &pamh);
    printf("pam_start(%s, %s) = %d (%s)\n", service, user, rc, pam_strerror(NULL, rc));
    if (rc != PAM_SUCCESS)
        return 1;

    rc = pam_authenticate(pamh, 0);
    printf("pam_authenticate    = %d (%s)\n", rc, pam_strerror(pamh, rc));

    switch (rc) {
    case PAM_SUCCESS:
        printf("verdict: STACK GRANTS ACCESS\n");
        break;
    case PAM_AUTHINFO_UNAVAIL:
    case PAM_MODULE_UNKNOWN:
        printf("verdict: unavailable (kscreenlocker hides this authenticator)\n");
        break;
    default:
        printf("verdict: failed (kscreenlocker reports a generic failure)\n");
        break;
    }

    const int end_rc = pam_end(pamh, rc);
    if (end_rc != PAM_SUCCESS)
        fprintf(stderr, "pam_end = %d\n", end_rc);

    return rc == PAM_SUCCESS ? 0 : 1;
}
