//! Stable static instrumentation vocabulary, shared with generated coverage tests.
macro_rules! table {
    ($name:ident,$doc:literal; $($variant:ident=$id:literal),+ $(,)?)=> {
        #[doc=$doc]
        #[derive(Clone,Copy,Debug,Eq,PartialEq)]
        #[repr(u16)]
        pub enum $name {$(#[doc=stringify!($variant)] $variant=$id),+}
        impl $name {
            /// Every defined value, in stable wire order.
            pub const ALL:&'static [Self]=&[$(Self::$variant),+];
            /// Static, host-authored label; never guest input.
            pub const fn name(self)->&'static str {match self {$(Self::$variant=>stringify!($variant)),+}}
        }
    }
}
table! {State,"Thread state ID. Exact residence totals and inclusive spans are separate.";
Unknown=0,Running=1,Mutex=2,Condvar=3,Receive=4,EventWait=5,HostIoctl=6,Sleep=7,
Join=8,FileLock=9,Trap=10,Relay=11,Drain=12,MapApply=13,DisplayCopy=14,FrameSend=15,Rpc=16,Idle=17,Init=18}
table! {Site,"Initial reserved Site families; integration must assign individual wait expressions.";
Unknown=0,Init=1,TrapDispatch=2,WorkerPark=3,RelayCompletion=4,DrainerPark=5,VaManagerPark=6,
HostRmIoctl=7,MappingApply=8,DisplayWake=9,DisplayCopy=10,BrokerReceive=11,BrokerSend=12,
HeldReply=13,ShutdownJoin=14,ProfAggregator=15}
table! {Kind,"Outstanding obligation kind. Bounds are advisory only.";
DoorbellCompletion=0,GpPutObserved=1,HeldReply=2,MmuInvalidate=3,MappingApply=4,FlipCompletion=5,
FrameRequested=6,BrokerCredit=7,ForwardedRm=8,InterruptAck=9,ChannelBirth=10,ChannelTeardown=11}
impl Kind {
    /// Provisional age threshold, nanoseconds; never a timeout/decision input.
    pub const fn bound_ns(self) -> u64 {
        match self {
            Self::GpPutObserved
            | Self::HeldReply
            | Self::MmuInvalidate
            | Self::MappingApply
            | Self::ForwardedRm => 12_000_000,
            Self::FlipCompletion => 50_000_000,
            Self::ChannelBirth | Self::ChannelTeardown => 1_000_000_000,
            _ => 100_000_000,
        }
    }
}
