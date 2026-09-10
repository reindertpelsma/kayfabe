# The `WaitKind` ruling — responsiveness, not duration

**STATUS: LIVE (2026-09-09).** The rule `kayfabe-util`'s `WaitKind` is the mechanism for.

⊘ This file exists because the owner's ruling names OS readiness machinery by name, and the
pure-logic crates are gated against doing that — *"no host descriptor types, no syscall names,
not even in comments"* (`l1_concurrency.md` §6.2). The quote is load-bearing and must stay
verbatim, so it lives here and `WaitKind` refers to it. ⚠ Rewording an owner's words to satisfy
a lint would be the wrong repair; moving them to where they are allowed is the right one.

## The ruling, verbatim

> **Owner, 2026-09-09:** *"for a coordinator the only long sleep you should encounter is a
> poll/epoll waiting on multiple fds (thats allowed) and is responsive to new input. The only
> thing is that its therefore responsive always, rather than block."*

And, from 2026-09-10, the policy it generalises to:

> *"I actually would say that we need to avoid blocking calls as much as possible in our code
> except epoll, since each blocking call is an unusable thread. Isolate can't get around it
> since ioctls of nvidia are blocking and you can't poll many of them, so there the workers are
> justified. My ideas is that for most function calls ensure its first resumable, so async.
> Then whenever you have a call that returns a fd you can wait on, the thread can put it in its
> epoll loop and simultaneously wait for that fd as well wait for input to remain responsive."*

## What it refuted

It **refutes the duration rule this module shipped an hour earlier**. A healthy idle multiplexed
wait parked for five seconds would have been reported as the worst offender in the system, while
a 900 µs uninterruptible device call — the actually harmful thing — passed. ★ **A metric that is
loudest where the design is most correct is worse than no metric.**

⇒ The question is not *"how long did you sleep"* but **"could new work have woken you"**.

## The one refinement, and it is not pedantry

*"Reader threads can sleep to wait for an operation"* — waiting for AN OPERATION TO COMPLETE is
the **bad** case: for that duration the thread is deaf to everything else. What is safe is
waiting for **events**, with that operation's completion multiplexed *alongside* new input on
the same primitive. The two look identical in a stack trace and behave oppositely.

⇒ Which is why a responsive wait **must name what can wake it**. A wait that cannot name its
wake source is not responsive; it is optimistic.

## Where the exception is

The isolate's device calls block and cannot be multiplexed, so worker threads there are the
justified exception rather than a failure of this rule. Everywhere else, a blocking call costs a
thread that could have been serving other work, and the shape to reach for is a resumable call
whose readiness arrives as one more source in an existing multiplexed wait.
