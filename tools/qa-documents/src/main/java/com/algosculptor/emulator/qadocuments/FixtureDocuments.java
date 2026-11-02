package com.algosculptor.emulator.qadocuments;

import android.database.Cursor;
import android.database.MatrixCursor;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract.Document;
import android.provider.DocumentsContract.Root;
import android.provider.DocumentsProvider;
import java.io.FileNotFoundException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.concurrent.atomic.AtomicBoolean;

/** Separate read-only QA APK. Only our own NROs, deliberately slow pipes and invalid fixtures. */
public final class FixtureDocuments extends DocumentsProvider {
    private static final String[] ROOT_COLUMNS={Root.COLUMN_ROOT_ID,Root.COLUMN_TITLE,Root.COLUMN_DOCUMENT_ID,Root.COLUMN_FLAGS,Root.COLUMN_MIME_TYPES};
    private static final String[] DOC_COLUMNS={Document.COLUMN_DOCUMENT_ID,Document.COLUMN_DISPLAY_NAME,Document.COLUMN_MIME_TYPE,Document.COLUMN_SIZE,Document.COLUMN_FLAGS};
    private static final String[] FILES={"slow-hello.nro","slow-ipc.nro","hello.nro","duplicate.nro","bad-opcode.nro","invalid.nro","module.nro","oversize.nro"};
    private static void add(MatrixCursor c,Object... values){
        Object[] row=new Object[c.getColumnCount()];
        for(int i=0;i<values.length;i+=2){int at=c.getColumnIndex((String)values[i]);if(at>=0)row[at]=values[i+1];}c.addRow(row);
    }
    @Override public boolean onCreate(){return true;}
    @Override public Cursor queryRoots(String[] projection){
        MatrixCursor c=new MatrixCursor(projection==null?ROOT_COLUMNS:projection);
        add(c,Root.COLUMN_ROOT_ID,"qa",Root.COLUMN_TITLE,"QA Documents",Root.COLUMN_DOCUMENT_ID,"root",Root.COLUMN_FLAGS,Root.FLAG_LOCAL_ONLY,Root.COLUMN_MIME_TYPES,"*/*");return c;
    }
    private void row(MatrixCursor c,String id)throws FileNotFoundException{
        if(id.equals("root")){add(c,Document.COLUMN_DOCUMENT_ID,id,Document.COLUMN_DISPLAY_NAME,"QA Documents",Document.COLUMN_MIME_TYPE,Document.MIME_TYPE_DIR,Document.COLUMN_SIZE,0,Document.COLUMN_FLAGS,0);return;}
        boolean found=false;for(String name:FILES)if(name.equals(id))found=true;
        if(!found)throw new FileNotFoundException("Unknown fixture");
        long size=32L*1024*1024+1;
        if(!id.equals("oversize.nro")){try(InputStream input=getContext().getAssets().open(id)){size=input.available();}catch(Exception e){throw new FileNotFoundException(e.toString());}}
        add(c,Document.COLUMN_DOCUMENT_ID,id,Document.COLUMN_DISPLAY_NAME,id,Document.COLUMN_MIME_TYPE,"application/octet-stream",Document.COLUMN_SIZE,size,Document.COLUMN_FLAGS,0);
    }
    @Override public Cursor queryDocument(String id,String[] projection)throws FileNotFoundException{
        MatrixCursor c=new MatrixCursor(projection==null?DOC_COLUMNS:projection);row(c,id);return c;
    }
    @Override public Cursor queryChildDocuments(String parent,String[] projection,String sort)throws FileNotFoundException{
        MatrixCursor c=new MatrixCursor(projection==null?DOC_COLUMNS:projection);if(!parent.equals("root"))throw new FileNotFoundException();for(String id:FILES)row(c,id);return c;
    }
    @Override public boolean isChildDocument(String parent,String child){return parent.equals("root")&&!child.equals("root");}
    @Override public ParcelFileDescriptor openDocument(String id,String mode,CancellationSignal signal)throws FileNotFoundException{
        if(!mode.equals("r"))throw new FileNotFoundException("Read-only test documents");
        boolean found=false;for(String name:FILES)if(name.equals(id))found=true;if(!found)throw new FileNotFoundException();
        try{
            ParcelFileDescriptor[] pipe=ParcelFileDescriptor.createPipe();AtomicBoolean cancelled=new AtomicBoolean();
            if(signal!=null)signal.setOnCancelListener(()->{cancelled.set(true);try{pipe[1].close();}catch(Exception ignored){}});
            new Thread(()->{
                try(OutputStream output=new ParcelFileDescriptor.AutoCloseOutputStream(pipe[1])){
                    if(id.equals("oversize.nro")){byte[] b=new byte[32768];long remaining=32L*1024*1024+1;while(remaining>0&&!cancelled.get()){int n=(int)Math.min(b.length,remaining);output.write(b,0,n);remaining-=n;}}
                    else try(InputStream input=getContext().getAssets().open(id)){
                        byte[] b=new byte[id.startsWith("slow-")?64:4096];int n;
                        while(!cancelled.get()&&(n=input.read(b))>=0){output.write(b,0,n);if(id.startsWith("slow-"))Thread.sleep(40);}
                    }
                }catch(Exception ignored){/* Closed/cancelled reader is expected in cancellation tests. */}
            },"QaDocumentPipe").start();return pipe[0];
        }catch(Exception e){throw new FileNotFoundException(e.toString());}
    }
}
