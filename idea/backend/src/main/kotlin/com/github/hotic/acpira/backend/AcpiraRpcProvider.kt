@file:Suppress("UnstableApiUsage")

package com.github.hotic.acpira.backend

import com.github.hotic.acpira.rpc.AcpiraBackendApi
import com.intellij.platform.rpc.backend.RemoteApiProvider
import fleet.rpc.remoteApiDescriptor

// Registers the shared Acpira RPC descriptor on backend-capable products; frontend-only installations intentionally resolve no provider.
internal class AcpiraRpcProvider : RemoteApiProvider {
    override fun RemoteApiProvider.Sink.remoteApis() {
        remoteApi(remoteApiDescriptor<AcpiraBackendApi>()) { AcpiraBackendApiImpl() }
    }
}
