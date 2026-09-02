# K1-K3 kernel port plan

This document records the kernel and core user-task work needed to carry the
K1, K2, and K3 assignments from `../trains` into this Rust workspace. It is an
implementation plan, not a claim that the current Rust kernel already provides
these interfaces.

The source comparison was made against `../trains` at `96464e3`, with the K1,
K2, and K3 milestone branches used where later train work obscures an earlier
design. The relevant milestone tips are `origin/K1-hotfix` (`4661964`),
`origin/k2` (`2bfa782`), and `origin/k3` (`dd2647c`).

## Scope

In scope:

- K1 task creation, identity, parent identity, yielding, exiting, and priority
  scheduling.
- K2 synchronous `Send`/`Receive`/`Reply` message passing and the name server
  needed by the K2 workloads.
- K3 `AwaitEvent`, timer interrupt delivery, the clock server and notifier,
  `Time`/`Delay`/`DelayUntil`, and an idle task.
- Small K1-K3 diagnostic workloads and commands that prove those facilities.

Out of scope:

- Train, switch, sensor, track, navigation, and Marklin behavior.
- UART interrupt-driven I/O servers and the later storage, console-display,
  resource-manager, and train service framework.
- SMP, an allocator, an MMU, and general user-memory isolation.

The RPS program and four clock clients are assignment diagnostics, not kernel
mechanisms. They should be ported only after their underlying K2 or K3 contract
is stable.

## Reference source map

The useful parts of the current `../trains` tree are narrowly contained:

| Concern | Reference paths |
| --- | --- |
| Kernel dispatch and lifecycle | `src/system/kernel/{kernel,exceptions,task_creation}.cpp` |
| Task storage and scheduling | `src/system/types/task/{task,registry,queue}.*` |
| Syscall wrappers and numbers | `src/system/{primitives,exceptions}.*` |
| K2 message passing | `src/system/kernel/message_passing.cpp`, `src/system/types/task/message.*` |
| K2 name server | `src/tasks/core/name/server.*` |
| K3 events and IRQ routing | `src/system/kernel/events.cpp`, `src/system/types/event/registry.*`, `src/system/kernel/exceptions.cpp` |
| K3 clock and idle tasks | `src/tasks/core/clock/server.*`, `src/tasks/core/idle/task.*` |
| Assignment diagnostics | `src/tasks/k1`, `src/tasks/k2`, `src/tasks/k3`, and `src/tasks/commands/k{1,2,3}.h` |

Everything under `src/tasks/track`, along with Marklin services and the later
application service graph, is intentionally outside this porting pass.

## Milestone dependency map

| Milestone | Kernel mechanisms | Minimal user tasks | Proof workload |
| --- | --- | --- | --- |
| Existing base | EL0 exception frames, context save/restore, static task slots, stable priority queue, `Yield`, `Exit`, one-shot timer/GIC support | EL1 polling command runtime | Cooperative-yield demo |
| K1 | EL0 `Create`, `MyTid`, `MyParentTid`; child creation while the scheduler is running; parent metadata; task cleanup | Low-priority first user task or shell | Two priority-1 and two priority-3 children, each yielding once |
| K2 | `Send`, `Receive`, `Reply`; send-, receive-, and reply-blocked states; per-task IPC metadata | Name server | RPS plus send-first/receive-first benchmarks |
| K3 | `AwaitEvent`; event wait registry; timer IRQ-to-event routing; interrupt-time rescheduling; periodic tick | Clock server, notifier, idle task | Four clock clients using `(delay, count)` pairs `(10,20)`, `(23,9)`, `(33,6)`, `(71,3)` |

Each row depends on the row above it. In particular, the K3 notifier is a K2
client: it waits for a hardware event and then sends a tick message to the
clock server.

## Existing Rust baseline and gaps

The current code has a strong architecture-level base:

- `crates/kernel/src/asm/exceptions.S` saves and restores the full general,
  SIMD, thread-pointer, and relevant system-register context.
- `crates/kernel/src/task.rs` owns pinned 8 KiB task stacks.
- `crates/kernel/src/scheduler` implements stable priority scheduling: lower
  numeric priorities run first and equal priorities are FIFO.
- `crates/kernel/src/svc.rs` and `CurrentTask` implement `Yield` and `Exit`.
- `crates/kernel/src/interrupts.rs` can claim and complete the Arm physical
  timer interrupt.

The current scheduler is still a demo scheduler rather than the K1-K3 kernel:

- Tasks can only be created from EL1 before `run_tasks`; creation is rejected
  while the scheduler is running.
- Every task is currently a root task. The descriptor has a parent field, but
  normal creation never populates it.
- Only `Ready` and `Running` task states exist.
- The scheduler returns to EL1 when all tasks exit. A K3 system instead keeps
  the name server, clock server, notifier, idle task, and user shell alive.
- User contexts start with IRQ masked. K3 user execution must restore a state
  in which IRQ delivery is enabled after initialization.
- Timer interrupt handling only supports its boot diagnostic. It does not
  wake event waiters, re-arm a 10 ms tick, save/requeue the interrupted task,
  or select another task.
- The command runtime runs at EL1 and polls the UART. It eventually needs to
  become, or be replaced by, a low-priority user task so kernel services and
  application tasks share one scheduling model.

## Shared kernel model

### Task descriptor and states

Keep task storage fixed-capacity and pinned. Extend each descriptor with the
metadata and IPC state needed by the milestones:

```text
Task
  id, parent_id, priority, state
  saved register context
  pinned stack
  outbound message/reply buffers       (K2)
  FIFO of waiting sender IDs            (K2)
  pending Receive destination           (K2)
  awaited event, if any                 (K3)
```

The required states are:

| State | Meaning | Eligible for ready queue? |
| --- | --- | --- |
| `Ready` | Runnable but not selected | Yes |
| `Running` | Current EL0 task | No |
| `SendBlocked` | `Send` is waiting for a matching `Receive` | No |
| `ReplyBlocked` | Message was received; `Send` is waiting for `Reply` | No |
| `ReceiveBlocked` | `Receive` is waiting for a sender | No |
| `EventBlocked` | `AwaitEvent` is waiting for an interrupt/event | No |

An empty slot represents an exited task; a separate `Exited` state is not
needed if slot cleanup is immediate. The ready queue must contain a task at
most once, and only while it is `Ready`.

### Exception and syscall invariant

Every syscall and scheduling IRQ follows the same transaction:

1. The assembly entry stub captures the outgoing EL0 context in an
   `ExceptionFrame` and masks IRQ while kernel state is mutated.
2. Decode the SVC or claimed interrupt.
3. Apply exactly one task-state transition and write syscall results to the
   saved `x0` of whichever tasks complete.
4. Put newly runnable tasks in the stable ready queue.
5. Select the highest-priority ready task and restore its context into the
   exception frame.
6. `eret` restores the selected task's interrupt mask along with its context.

All user syscalls are scheduling points, including `MyTid` and
`MyParentTid`. This matches the final reference kernel and keeps scheduling
behavior simple and observable.

The Rust implementation should keep syscall numbers private to the kernel
crate and expose typed safe wrappers. Treat raw register values and user
pointers as unsafe only at the exception-dispatch boundary.

### Task IDs and cleanup

`../trains` uses the task-slot index as its TID and recycles it on exit. The
Rust port may retain that policy initially, but lookup must reject vacant
slots. The final C++ `Registry::Get` returns a pointer for any in-range slot,
including exited slots; that is a reference bug, not a contract to preserve.

When a task exits, remove it from every kernel-owned queue or registry. Before
supporting arbitrary task death, define what happens to senders blocked on
that task. The safe policy is to wake them with `TaskNotFound`; the assignment
workloads avoid this edge case, so it can be implemented after the basic K2
path if it is explicitly tracked.

## K1: task lifecycle and scheduling

### User API

Add user-mode wrappers with the semantics below. Existing EL1 bootstrap
creation may remain as a separate API.

| Operation | Inputs | Result |
| --- | --- | --- |
| `Create` | priority, task entry | Child TID, or a typed invalid-priority/capacity error |
| `MyTid` | none | Current TID |
| `MyParentTid` | none | Creator's TID; the bootstrap task has no parent |
| `Yield` | none | Current task moves to the back of its priority level |
| `Exit` | none | Current slot is released and is never restored |

For the initial task, use `None` as the internal parent representation and
choose one documented wrapper result for compatibility. Do not silently
claim that TID 0 is its parent merely because the C++ reference did so.

Normalize the priority domain before exposing `Create`. The final reference
accepts signed values from 0 (highest) through `i32::MAX` (lowest), while the
current Rust `Priority` accepts every `u32` and defines `u32::MAX` as lowest.
Either Rust domain is workable, but the public wrapper, validation behavior,
and K1 error test must agree.

### Scheduler changes

- Permit `Create` while the scheduler is running.
- Allocate the child in its final pinned slot, initialize its context, set its
  parent to the current task, and enqueue it.
- Complete `Create` by placing the new TID in the parent's saved `x0`, making
  the parent ready, and rescheduling. A higher-priority child can therefore
  run before `Create` returns to its parent.
- `MyTid` and `MyParentTid` set saved `x0`, ready the caller, and reschedule.
- `Yield` readies the caller and reschedules without changing other state.
- `Exit` vacates the current slot and dispatches another task. If no task is
  ready during a finite diagnostic run, restore the saved EL1 context; in the
  full K3 runtime, the idle task guarantees a runnable fallback.
- Decide whether task entry functions must explicitly call `Exit` or return
  through a kernel-owned trampoline. The reference uses a wrapper that calls
  `Exit` after the user function returns. A Rust trampoline gives the same
  ergonomic and cleanup guarantee and is preferred for ordinary `fn()` task
  bodies; the existing `fn(TaskId) -> !` entry ABI can remain if explicit exit
  is chosen instead.

### K1 acceptance checks

- A child observes its own TID and the creating task's TID.
- Invalid priorities and full task storage return errors without corrupting
  the ready queue.
- A newly created higher-priority child runs before its parent resumes.
- Equal-priority tasks remain FIFO across repeated yields.
- Exiting tasks are never selected again, and their slots can be reused.
- The reference K1 workload produces two log lines per child, separated by a
  yield, for two priority-1 and two priority-3 children.

## K2: synchronous message passing

### User API and ABI

The syscall boundary mirrors the reference's AArch64 argument registers:

| Operation | Arguments | Successful result in `x0` |
| --- | --- | --- |
| `Send` | `x0=tid`, `x1=msg`, `x2=msg_len`, `x3=reply`, `x4=reply_capacity` | Reply's logical length |
| `Receive` | `x0=&sender_tid`, `x1=buffer`, `x2=capacity` | Message's logical length |
| `Reply` | `x0=tid`, `x1=reply`, `x2=reply_len` | Reply's logical length |

The logical length is the source length even if the destination buffer
truncates the copied bytes. Use byte slices in the safe Rust API; messages are
not C strings and the kernel should not add or copy a trailing NUL. A null
pointer is valid only when its associated length/capacity is zero.

Expose distinct typed errors internally, at least `TaskNotFound`,
`NotReplyBlocked`, and `QueueFull`. If compatibility wrappers use negative
integers, define the conversion in one place rather than spreading sentinel
values through scheduler logic.

### Rendezvous state machine

`Send(receiver, message, reply_buffer)` has two paths:

- If the receiver is `ReceiveBlocked`, copy the message immediately, write
  the sender TID and message length to the receiver, make the receiver ready,
  and move the sender directly to `ReplyBlocked`.
- Otherwise append the sender TID to the receiver's FIFO and move the sender
  to `SendBlocked`. The sender's task slot retains the borrowed message and
  reply buffer metadata until the transaction completes.

`Receive(sender_out, buffer)` also has two paths:

- If the incoming-sender FIFO is non-empty, pop its oldest valid sender, copy
  the message, write its TID and logical length to the receiver, change the
  sender from `SendBlocked` to `ReplyBlocked`, and make the receiver ready.
- Otherwise store the receive destinations in the receiver's task slot and
  move it to `ReceiveBlocked`.

`Reply(sender, reply)` succeeds only when the target is `ReplyBlocked`. It
copies into the reply buffer retained by the sender, writes the logical reply
length to both the sender's suspended `Send` and the replier's `Reply`, clears
the sender's outbound-message metadata, and makes both tasks ready.

In every path, the scheduler runs after the state transition. Priority, not
the identity of the syscall caller, decides which newly ready task runs next.

### Memory and lifetime rules

The C++ implementation retains user buffer pointers while tasks are blocked.
That works only because each blocked task remains alive and cannot reuse its
stack. Preserve the following invariants explicitly:

- A `Send` caller cannot resume or mutate its message/reply buffers before
  `Reply` completes.
- A `Receive` caller cannot resume or mutate its receive destinations before
  a sender is matched.
- IPC metadata is cleared on completion or task cleanup.
- Copy lengths use checked conversions and `min(source_len, capacity)`.
- Queue entries identify tasks by validated slot/TID rather than holding Rust
  references across later mutable scheduler operations.

With no MMU, the kernel cannot make arbitrary EL0 pointers trustworthy. Keep
all pointer decoding and copying in a small audited unsafe module so a future
address-space policy has one enforcement point.

### Name server and K2 diagnostics

The K2 name server is a regular highest-priority user task built on IPC. It
loops on `Receive` and supports:

- `RegisterAs(name)`: associate the sender TID with a name, replacing an
  existing association, then reply success.
- `WhoIs(name)`: reply with the registered TID or not-found.

Do not hard-code its TID as `1`, as `../trains` does. Record the TID returned
by bootstrap creation and provide it to clients through an explicit
bootstrapped handle or another documented mechanism.

After the IPC and name-server tests pass, port the RPS workload and the
send-first/receive-first benchmark as diagnostics. They belong in the
application crate, not the kernel crate, and introduce no new kernel API.

### K2 acceptance checks

- Send-first and receive-first rendezvous both work.
- Multiple senders are received FIFO.
- A sender stays blocked through receive and resumes only after reply.
- Message and reply buffers truncate safely while returning logical lengths.
- Sending to a vacant/out-of-range TID and replying to a task not in
  `ReplyBlocked` return distinct errors.
- A full sender queue fails without leaving the caller blocked or retaining
  stale buffer metadata.
- Name replacement and unknown-name lookup behave predictably.

## K3: events, interrupts, and clock tasks

### AwaitEvent and event registry

Add `AwaitEvent(EventId) -> event_data` as a user syscall. The handler
registers the current task for the event, moves it to `EventBlocked`, and
reschedules without re-enqueueing it.

The event registry is a fixed-capacity mapping from typed `EventId` values to
waiting task IDs. `../trains` wakes every waiter registered for an event and
clears that event's queue. Preserve that behavior initially, while validating
event IDs and rejecting duplicate registration by one task. Hardware and
kernel code signal events; the later public `SignalEvent` syscall in
`../trains` was added for train measurements and is outside K1-K3 scope.

On signal, each valid waiter receives the event data in saved `x0`, becomes
`Ready`, and is enqueued exactly once. Signaling an event with no waiters is a
no-op; events are edge notifications and are not buffered for future waiters.

### Timer interrupt scheduling

Use the existing Arm physical timer and GIC abstraction. K3 initialization
must:

- Enable the physical timer interrupt in the GIC.
- Program a 10 ms tick.
- Install the exception vectors before unmasking IRQ.
- Initialize new EL0 contexts with IRQ unmasked while leaving FIQ and other
  unsupported exception classes masked as intended.

The K3 branch of `../trains` uses BCM system-timer compare 1 instead. That
specific peripheral is not part of the assignment-facing contract; the Rust
kernel should keep its already working Arm physical timer and expose the same
10 ms clock-tick event semantics.

On each timer IRQ:

1. Save the interrupted task context.
2. Claim the GIC interrupt and identify the physical timer.
3. Cancel/acknowledge the timer source, complete the GIC claim, and program
   the next 10 ms deadline.
4. Signal the clock-tick event and ready all waiters.
5. Make the interrupted `Running` task `Ready` and enqueue it unless the
   interrupt path itself changed that task to a blocked/exited state.
6. Dispatch the highest-priority ready task before returning from the IRQ.

The claim loop should drain all pending interrupts, as the final reference
kernel does. Unknown interrupt IDs should be diagnosed and completed safely,
not leave the GIC wedged.

### Clock server and notifier

These are user tasks, not kernel features:

- The highest-priority clock server owns a monotonic `Tick` counter and a
  min-priority queue ordered by wake tick.
- Its dedicated notifier loops over `AwaitEvent(clock_tick)` and
  `Send(clock_server, tick)`. The server has higher scheduling priority and
  replies to the notifier after accounting for the tick, preventing the
  notifier from racing ahead.
- `Time` sends a request and receives the current tick.
- `Delay(ticks)` leaves its sender reply-blocked until `current_tick` reaches
  the tick observed at the request plus `ticks`.
- `DelayUntil(tick)` leaves its sender reply-blocked until `current_tick`
  reaches `tick`.
- At each tick, the server replies to every request whose target is due.

Reject negative delay values in the user wrapper. Define zero/past delays to
reply immediately; `../trains` waits until the next tick because it checks the
delay queue only from its tick handler, which is an implementation artifact
rather than desirable API behavior. Use checked or saturating tick arithmetic
and specify the chosen overflow behavior.

### Idle task

Create one task at `Priority::LOWEST` that executes `wfi` in a loop. For the
K3 deliverable it may measure time spent as the fallback task and periodically
report an idle percentage. Keep accounting outside the scheduler unless a
later metric requires exact context-switch timestamps. The first correct
version only needs to guarantee that the kernel always has a runnable task and
that `wfi` wakes on the timer IRQ.

### K3 acceptance checks

- A task blocked in `AwaitEvent` is absent from the ready queue and wakes with
  the signaled data.
- Timer ticks continue after the boot-time interrupt diagnostic; at least
  hundreds of sequential ticks are observed without a lost re-arm.
- An IRQ preempts a running lower-priority task and a newly readied
  higher-priority notifier runs next.
- `Time` is monotonic; `Delay` and `DelayUntil` never return early.
- Multiple requests due on the same tick all wake in stable order.
- Zero/past delays return immediately and negative delays return an error.
- The four reference clock clients complete the pairs `(10,20)`, `(23,9)`,
  `(33,6)`, and `(71,3)` in the expected priority-driven interleaving.
- The idle task runs only when no higher-priority task is ready.

## Recommended implementation sequence

1. **Separate finite diagnostics from the resident runtime.** Preserve a way
   for boot self-tests to run and return to EL1, but add a non-returning kernel
   start path for the K1-K3 user environment.
2. **Finish K1 in the existing scheduler.** Add child creation, identity
   syscalls, parent metadata, the chosen exit trampoline policy, and K1 tests.
3. **Introduce one scheduler-owned transition API.** Centralize lookup,
   return-value writes, readying, blocking, exit cleanup, and dispatch before
   adding more states.
4. **Add K2 IPC storage and syscalls.** Test state transitions directly, then
   through EL0 SVC calls, before adding the name server.
5. **Move the shell/bootstrap into user tasks.** Start the name server first,
   then the low-priority shell; retain polling console I/O for now.
6. **Add the event registry and interrupt-time scheduling.** Convert the
   current one-shot timer diagnostic path into the periodic K3 tick only after
   scheduler IRQ invariants are tested.
7. **Add the clock server, notifier, and idle task.** Then port the K3 clock
   clients as the end-to-end test.
8. **Add K2 RPS and benchmark diagnostics.** These can follow the kernel work
   and must not pull in later train-service abstractions.

Likely file ownership in this workspace:

| Area | Existing or proposed location |
| --- | --- |
| Shared `TaskId`, `Priority`, `EventId`, `Tick` | `crates/primitives` |
| Syscall numbers and raw ABI decoding | `crates/kernel/src/svc.rs` plus a small user-pointer/IPC module |
| Task metadata, IPC fields, stack/context | `crates/kernel/src/task.rs` |
| State transitions and selection | `crates/kernel/src/scheduler` |
| K2 message rendezvous | new `crates/kernel/src/ipc.rs` or scheduler submodule |
| K3 event wait registry | new `crates/kernel/src/events.rs` |
| IRQ-to-event routing and periodic timer | `crates/kernel/src/interrupts.rs` |
| Safe user-facing syscall wrappers | kernel public API, alongside `CurrentTask` |
| Name server, clock server, idle task, K1-K3 workloads | `crates/application`, separated from future train modules |

## Reference behavior not to copy blindly

The C++ implementation proves the overall design, but several details should
not become Rust contracts accidentally:

- `Registry::Get` can return an exited task for any in-range TID.
- Some revisions copy `length + 1` for string terminators even though syscall
  lengths describe bytes; Rust IPC should be byte-oriented.
- The name and clock clients rely on hard-coded/global server TIDs.
- Event registration can allocate queues dynamically and does not consistently
  propagate a full-queue failure.
- `AwaitEvent` accepts arbitrary event IDs without validation.
- `Delay(0)` and a past `DelayUntil` wait for the next tick.
- Later revisions expose `SignalEvent` to user tasks for train measurements;
  K3 only needs kernel/interrupt event signaling.
- Later UART, storage, display, service, and train code is not a prerequisite
  for K1-K3.

Keeping these distinctions explicit lets the Rust port reproduce the tested
K1-K3 semantics while preserving its stronger type, ownership, and fixed-
capacity design.
