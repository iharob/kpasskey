# kpasskey — build and install
#
# PREFIX is where things end up (/usr by default, since these are system integration
# points: systemd units, a PAM module and a Plasma KCM all live under /usr).
# DESTDIR is the usual staging root for packaging: `make install DESTDIR=/tmp/pkg`.

PREFIX     ?= /usr
DESTDIR    ?=

BINDIR     := $(PREFIX)/bin
LIBDIR     := $(PREFIX)/lib
SYSTEMDDIR := $(LIBDIR)/systemd/system
USERDIR    := $(LIBDIR)/systemd/user
TMPFILESDIR:= $(LIBDIR)/tmpfiles.d
SECURITYDIR:= $(LIBDIR)/security
DATADIR    := $(PREFIX)/share/kpasskey

CARGO      ?= cargo
CC         ?= cc
CMAKE      ?= cmake
INSTALL    ?= install

DAEMON_BIN := daemon/target/release/kpk-spike
PAM_MODULE := pam/tests/pam_kpk_spike.so
KCM_BUILD  := kcm/build

# `sudo make install` runs every recipe as root, so a bare `systemctl --user` would
# reach root's session bus instead of the desktop user's and silently enable the unit
# for the wrong account. SUDO_USER/SUDO_UID are the only way back to who invoked it.
LOGIN_USER := $(if $(SUDO_USER),$(SUDO_USER),$(shell id -un))
LOGIN_UID  := $(if $(SUDO_UID),$(SUDO_UID),$(shell id -u))

USER_SYSTEMCTL := runuser -u $(LOGIN_USER) -- \
    env XDG_RUNTIME_DIR=/run/user/$(LOGIN_UID) systemctl --user

.PHONY: all daemon pam kcm install install-daemon install-pam install-kcm \
        activate deactivate uninstall clean

all: daemon pam kcm

daemon:
	$(CARGO) build --release --locked --manifest-path daemon/Cargo.toml

# A warning is a defect; this is the same gate the Rust and Kotlin halves use.
pam: $(PAM_MODULE)

$(PAM_MODULE): pam/tests/pam_kpk_spike.c
	$(CC) -shared -fPIC -O2 -Wall -Wextra -Werror -D_FORTIFY_SOURCE=3 \
	    -o $@ $< -lpam

kcm:
	$(CMAKE) -S kcm -B $(KCM_BUILD) -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$(PREFIX)
	$(CMAKE) --build $(KCM_BUILD)

install: install-daemon install-pam install-kcm
ifeq ($(strip $(DESTDIR)),)
	@$(MAKE) --no-print-directory activate
else
	@echo "DESTDIR set: staged only, live system untouched (run 'make activate' there)"
endif

install-daemon: daemon
	$(INSTALL) -Dm755 $(DAEMON_BIN) $(DESTDIR)$(BINDIR)/kpk-spike
	$(INSTALL) -Dm644 daemon/data/systemd/kpasskey-fido.service \
	    $(DESTDIR)$(SYSTEMDDIR)/kpasskey-fido.service
	$(INSTALL) -Dm644 daemon/data/systemd/kpasskey.service \
	    $(DESTDIR)$(USERDIR)/kpasskey.service
	$(INSTALL) -Dm644 daemon/data/tmpfiles/kpasskey.conf \
	    $(DESTDIR)$(TMPFILESDIR)/kpasskey.conf
	# The units ship pointing at the build tree so a rebuild takes effect on restart;
	# rewrite that to the installed binary.
	sed -i 's|^ExecStart=.*/target/release/kpk-spike|ExecStart=$(BINDIR)/kpk-spike|' \
	    $(DESTDIR)$(SYSTEMDDIR)/kpasskey-fido.service \
	    $(DESTDIR)$(USERDIR)/kpasskey.service

install-pam: pam
	$(INSTALL) -Dm755 $(PAM_MODULE) $(DESTDIR)$(SECURITYDIR)/pam_kpk_spike.so
	# Shipped as a reference, never installed into /etc/pam.d: silently rewriting an auth
	# stack has "cannot log in" as its failure mode. Install it yourself, with a root shell
	# already open on another TTY.
	$(INSTALL) -Dm644 pam/tests/polkit-1.spike $(DESTDIR)$(DATADIR)/polkit-1.example

install-kcm: kcm
	DESTDIR=$(DESTDIR) $(CMAKE) --install $(KCM_BUILD) --prefix $(PREFIX)

activate:
	@set -e; \
	if [ "$$(id -u)" -ne 0 ]; then \
	    echo "activate: needs root — run 'sudo make install'"; exit 1; \
	fi; \
	systemd-tmpfiles --create $(TMPFILESDIR)/kpasskey.conf; \
	systemctl daemon-reload; \
	systemctl enable --now kpasskey-fido.service; \
	if [ "$(LOGIN_USER)" = root ]; then \
	    echo "activate: invoked by root directly, so there is no desktop user to enable"; \
	    echo "          the session unit for. As that user: systemctl --user enable --now kpasskey.service"; \
	else \
	    $(USER_SYSTEMCTL) daemon-reload; \
	    $(USER_SYSTEMCTL) enable --now kpasskey.service; \
	fi; \
	echo; \
	echo "PAM is deliberately not wired up: rewriting an auth stack has 'cannot log in'"; \
	echo "as its failure mode. With a root shell already open on another TTY, run"; \
	echo "    sudo make install-polkit"

# Separate target, never a dependency of install, for exactly the reason it prints.
.PHONY: install-polkit
install-polkit:
	@if [ -f /etc/pam.d/polkit-1 ] && ! grep -q pam_kpk /etc/pam.d/polkit-1; then \
	    echo "/etc/pam.d/polkit-1 exists and is not ours; refusing to overwrite it"; \
	    echo "back it up and remove it first if you mean to replace it"; exit 1; \
	fi
	$(INSTALL) -m0644 $(DATADIR)/polkit-1.example /etc/pam.d/polkit-1

deactivate:
	@set -e; \
	if [ "$$(id -u)" -ne 0 ]; then \
	    echo "deactivate: needs root"; exit 1; \
	fi; \
	systemctl disable --now kpasskey-fido.service || true; \
	if [ "$(LOGIN_USER)" != root ]; then \
	    $(USER_SYSTEMCTL) disable --now kpasskey.service || true; \
	fi

uninstall: deactivate
	rm -f $(DESTDIR)$(BINDIR)/kpk-spike \
	      $(DESTDIR)$(SECURITYDIR)/pam_kpk_spike.so \
	      $(DESTDIR)$(SYSTEMDDIR)/kpasskey-fido.service \
	      $(DESTDIR)$(USERDIR)/kpasskey.service \
	      $(DESTDIR)$(TMPFILESDIR)/kpasskey.conf
	rm -rf $(DESTDIR)$(DATADIR) /run/kpasskey
	# CMake is the only thing that knows where it put the KCM on this machine.
	if [ -r $(KCM_BUILD)/install_manifest.txt ]; then \
	    xargs -r rm -f < $(KCM_BUILD)/install_manifest.txt; \
	fi
	# Only ours, and only when it is ours; the vendor stack under /usr/lib/pam.d takes over.
	if grep -q pam_kpk /etc/pam.d/polkit-1 2>/dev/null; then rm -f /etc/pam.d/polkit-1; fi
	systemctl daemon-reload

clean:
	$(CARGO) clean --manifest-path daemon/Cargo.toml
	rm -f $(PAM_MODULE)
	rm -rf $(KCM_BUILD)
