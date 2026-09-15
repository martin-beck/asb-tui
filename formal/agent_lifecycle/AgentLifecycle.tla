------------------------------ MODULE AgentLifecycle ------------------------------
EXTENDS Naturals, Sequences

CONSTANTS Disconnected, Ready, Refreshing, Installing, Installed, Cancelling,
          Removing, Reconnecting, Stale, Failed

VARIABLES state, generation, progress, total

Init ==
  /\ state = Disconnected
  /\ generation = ""
  /\ progress = 0
  /\ total = 0

Refresh ==
  /\ state \in {Disconnected, Ready, Installed, Failed}
  /\ state' = Refreshing
  /\ UNCHANGED <<generation, progress, total>>

CatalogAccepted ==
  /\ state = Refreshing
  /\ state' = Ready
  /\ generation' = generation
  /\ UNCHANGED <<progress, total>>

Install ==
  /\ state = Ready
  /\ total' \in 1..1000000
  /\ state' = Installing
  /\ progress' = 0
  /\ UNCHANGED generation

Progress ==
  /\ state = Installing
  /\ progress' \in progress..total
  /\ state' = Installing
  /\ UNCHANGED <<generation, total>>

InstallSuccess ==
  /\ state = Installing
  /\ progress = total
  /\ state' = Installed
  /\ UNCHANGED <<generation, progress, total>>

Cancel ==
  /\ state = Installing
  /\ state' = Cancelling
  /\ UNCHANGED <<generation, progress, total>>

Cancelled ==
  /\ state = Cancelling
  /\ state' = Disconnected
  /\ UNCHANGED <<generation, progress, total>>

Remove ==
  /\ state = Installed
  /\ state' = Removing
  /\ UNCHANGED <<generation, progress, total>>

RemoveSuccess ==
  /\ state = Removing
  /\ state' = Reconnecting
  /\ UNCHANGED <<generation, progress, total>>

Reconnect ==
  /\ state \in {Reconnecting, Ready, Installed}
  /\ state' = Reconnecting
  /\ UNCHANGED <<generation, progress, total>>

Failure ==
  /\ state = Installing
  /\ state' = Failed
  /\ UNCHANGED <<generation, progress, total>>

Next == Refresh \/ CatalogAccepted \/ Install \/ Progress \/ InstallSuccess
     \/ Cancel \/ Cancelled \/ Remove \/ RemoveSuccess \/ Reconnect \/ Failure

Spec == Init /\ [][Next]_<<state, generation, progress, total>>

TypeInvariant ==
  /\ state \in {Disconnected, Ready, Refreshing, Installing, Installed,
                 Cancelling, Removing, Reconnecting, Stale, Failed}
  /\ progress \in 0..1000000
  /\ total \in 0..1000000
  /\ progress <= total

=============================================================================
