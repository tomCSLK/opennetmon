# Sign-in, "My account" and user administration.

## Sign-in

login-title = Sign in
login-tagline = Network monitoring
login-submit = Sign in
login-free-software = opennetmon is free software under the AGPL-3.0
login-failed = Wrong user name or password.
login-busy = Too many sign-ins are being checked right now. Please try again in a few seconds.
login-throttled = Too many failed sign-in attempts. Please try again in { $wait }.
login-signed-out-idle =
    { $minutes ->
        [one] You were signed out after { $minutes } minute without activity.
       *[other] You were signed out after { $minutes } minutes without activity.
    }
login-wait-seconds =
    { $seconds ->
        [one] { $seconds } second
       *[other] { $seconds } seconds
    }
login-wait-minutes =
    { $minutes ->
        [one] { $minutes } minute
       *[other] { $minutes } minutes
    }

## Checks of user name and password

username-length = The user name must be 3 to 32 characters long.
username-chars = Allowed are letters, digits and . _ -
username-taken = This user name is already taken.
password-too-short = The password must be at least 10 characters long.
password-too-long = The password must not be longer than 128 characters.
password-mismatch = The two passwords do not match.

## My account

account-language-hint = “Same as the browser” follows the browser's language setting; without a matching language, German is used.
account-password-title = Change password
account-current-password = Current password
account-password-hint = 10 to 128 characters. Your other sessions (e.g. on your phone) are signed out; this one stays signed in.
account-too-many-attempts = Too many wrong passwords. Please try again later.
account-current-wrong = The current password is wrong.
account-password-unchanged = The new password must differ from the current one.

## User administration

users-subtitle = Administrators may change everything, viewers may only look.
users-created = Created
users-sessions = Sessions
users-me = (you)
users-stays-signed-in = stays signed in
users-new = New user
users-new-hint = User name 3 to 32 characters (letters, digits, . _ -), password 10 to 128 characters. Pass the password on to the person in a secure way.
users-create = Create user
user-role-missing = Please choose a role.
user-role-hint = Applies immediately, also in open sessions. The last administrator stays an administrator.
user-last-admin = This is the last administrator. Create another administrator first.
user-password-title = Set password
user-password-hint-me = Your other sessions are ended; this one stays signed in.
user-password-hint-other = All sessions of { $name } are ended.
user-delete = Delete user
user-delete-confirm = Really delete { $name } (signs out all sessions)
user-delete-self = You cannot delete your own account; another administrator has to do that.
user-idle-title = Automatic sign-out
user-stay-signed-in = Exception: stay signed in (e.g. for a wall display)
user-idle-hint =
    { $minutes ->
        [one] Without the tick, the user is signed out after { $minutes } minute without a click or key press
       *[other] Without the tick, the user is signed out after { $minutes } minutes without a click or key press
    }; the automatic refresh does not count as activity. With the tick, the user stays signed in until the session expires after { $hours } hours. Meant only for displays nobody works at; applies at once, also to open sessions.
user-idle-off = The automatic sign-out is switched off in the configuration (session_idle_minutes = 0). The exception only takes effect once it is switched on.
