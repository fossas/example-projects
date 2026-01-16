package com.example.app.utils;

import java.util.Map;
import java.net.URI;
import java.net.URL;
import org.dom4j.Document;
import org.dom4j.DocumentException;
import org.dom4j.jaxb.JAXBReader;
import com.google.common.io.Files;
import com.google.common.base.Charsets;
import java.io.File;

public class ContextReader 
{
    public static Document parseWithCtx(URL url) throws DocumentException, java.io.IOException {
        File addrFile = new File("addr.txt");
        String addrCtx = Files.toString(addrFile, Charsets.UTF_8);

        JAXBReader reader = new JAXBReader(addrCtx);
        Document document = reader.read(url);
        return document;
    }
}
