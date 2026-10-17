# Kernel exit recovery

The desktop backend automatically recovers kernels that it successfully started
or adopted. An unsuccessful initial launch does not enable recovery. An observed
exit is unexpected unless an intentional stop, update, or application shutdown
has disarmed recovery; the exit code alone does not determine intent.

## Runtime cost

Recovery reuses `CoreProcessState::managed_pid()` calls from existing status
consumers, including the background proxy monitor. It adds no polling interval,
health request, process scan, or log watcher during normal operation. The recovery
task sleeps on a `Notify`; retry timers and blocking startup work exist only while
handling an exit. Healthy status serialization uses one atomic flag read and omits
the recovery field. Detection latency follows the existing consumers' cadence
(the proxy monitor normally waits 15 seconds between runs), not an immediate OS
exit notification. The desktop application must remain running.

## Recovery policy

- Delays are 2, 5, and 15 seconds after detection or a failed recovery attempt.
- At most three attempts are allowed in a rolling five-minute window. Successful
  restarts preserve this budget, preventing a process that repeatedly starts and
  crashes from restarting indefinitely.
- Exhaustion pauses recovery until a manual start or restart. Stable running
  naturally ages attempts out when the next exit is observed.
- Manual stop and shutdown invalidate delayed attempts. Updates cancel the old
  attempt and preserve the intended running state for the existing restore path.
- The existing operation lock serializes recovery with manual commands and
  updates. A generation check after acquiring that lock rejects stale work.
- Recovery never stops a live process because a port check timed out. It uses the
  existing startup path and its readiness checks only when a restart is needed.

The home page displays recovery progress and failure details. Waiting recovery
can be canceled with Stop. Exit information and startup failures also go through
the existing diagnostic logging. Persistent recovery metadata is not written to
the usage database or configuration files.
